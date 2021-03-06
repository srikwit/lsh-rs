use crate::activations::Activation;
use crate::{activations, loss::Loss};
use fnv::FnvHashMap;
use lsh_rs::utils::create_rng;
use lsh_rs::{DataPoint, DataPointSlice, LshMem, SignRandomProjections};
use ndarray::prelude::*;
use ndarray_rand::rand_distr::{StandardNormal, Uniform};
use ndarray_rand::RandomExt;

pub type Weight = Array1<f32>;

pub struct MemArena {
    // the weights that constantly get updated
    pub pool: Vec<Weight>,
    // the original weights. They are only updated during re-hashing
    pool_backup: Vec<Weight>,
    // Freed indexes will be added to the free buffer.
    free: Vec<usize>,
}

impl MemArena {
    fn new() -> Self {
        MemArena {
            pool: vec![],
            pool_backup: vec![],
            free: vec![],
        }
    }

    fn add(&mut self, p: Weight) -> usize {
        match self.free.pop() {
            Some(idx) => {
                self.pool.insert(idx, p);
                idx
            }
            None => {
                self.pool.push(p);
                self.pool.len() - 1
            }
        }
    }

    fn get(&self, idx: &[usize]) -> Vec<&Weight> {
        idx.iter()
            .map(|&idx| self.pool.get(idx).expect("out of bounds idx"))
            .collect()
    }

    fn freeze(&mut self) {
        self.pool_backup = self.pool.clone();
    }
}

pub struct Network {
    pub w: Vec<Vec<u32>>,
    // biases for all layers
    lsh2bias: Vec<FnvHashMap<u32, f32>>,
    activations: Vec<Activation>,
    lsh_store: Vec<Option<LshMem<SignRandomProjections>>>,
    n_layers: usize,
    pub pool: MemArena,
    lsh2pool: Vec<FnvHashMap<u32, usize>>,
    dimensions: Vec<usize>,
    pub lr: f32,
    loss: String,
}

impl Network {
    ///   Example of one hidden layer with
    ///         - 2 inputs
    ///         - 3 hidden nodes
    ///         - 3 outputs
    ///
    ///         layers -->    [0,        1,          2]
    ///         ----------------------------------------
    ///         dimensions =  (2,     3,          3)
    ///         activations = (      ReLU,      Sigmoid)
    pub fn new(
        dimensions: Vec<usize>,
        activations: Vec<Activation>,
        n_projections: usize,
        n_hash_tables: usize,
        lr: f32,
        seed: u64,
        loss: &str,
    ) -> Self {
        let n_layers = dimensions.len();
        let mut w = Vec::with_capacity(n_layers);
        let mut pool = MemArena::new();
        let mut lsh2pool = Vec::with_capacity(n_layers);
        let mut lsh2bias = Vec::with_capacity(n_layers);
        let mut lsh_store = Vec::with_capacity(n_layers);
        let mut rng = create_rng(seed);

        // initialize layers
        for i in 0..(n_layers - 1) {
            let mut lsh2pool_i = FnvHashMap::default();
            let mut lsh2bias_i = FnvHashMap::default();

            let in_size = dimensions[i];
            let out_size = dimensions[i + 1];
            let n_perceptrons = in_size * out_size;
            let mut w_idx = Vec::with_capacity(n_perceptrons);

            let mut lsh = LshMem::new(n_projections, n_hash_tables, in_size)
                .srp()
                .unwrap();

            // initialize vectors per layer.
            for _ in 0..out_size {
                let p = Array1::random_using(in_size, StandardNormal, &mut rng);
                let p = p / (in_size as f32).powf(0.5);

                let lsh_idx = lsh.store_vec(p.as_slice().unwrap()).unwrap();
                let pool_idx = pool.add(p);
                lsh2pool_i.insert(lsh_idx, pool_idx);
                lsh2bias_i.insert(lsh_idx, 0.);
                w_idx.push(lsh_idx);
            }

            lsh2pool.push(lsh2pool_i);
            lsh2bias.push(lsh2bias_i);
            lsh_store.push(Some(lsh));
            w.push(w_idx);
        }
        pool.freeze();

        Network {
            w,
            lsh2bias,
            activations,
            lsh_store,
            n_layers,
            pool,
            lsh2pool,
            dimensions,
            lr,
            loss: loss.to_string(),
        }
    }

    fn get_pool_idx(&self, layer: usize, j: &[u32]) -> Vec<usize> {
        j.iter()
            .map(|idx| {
                *self.lsh2pool[layer]
                    .get(idx)
                    .expect("perceptron index out of bounds")
            })
            .collect()
    }

    pub fn get_weight_mut(&mut self, layer: usize, j: u32) -> &mut Weight {
        let pool_idx = self.get_pool_idx(layer, &[j])[0];
        self.pool
            .pool
            .get_mut(pool_idx)
            .expect("could not get mut perceptron")
    }

    pub fn get_bias_mut(&mut self, layer: usize, j: usize) -> &mut f32 {
        self.lsh2bias[layer]
            .get_mut(&(j as u32))
            .expect("could not get mut bias")
    }

    pub fn get_weight(&self, layer: usize, j: usize) -> &Weight {
        let pool_idx = *self.lsh2pool[layer]
            .get(&(j as u32))
            .expect("neuron index out of bounds");
        self.pool.pool.get(pool_idx).expect("could not get weight")
    }

    pub fn get_weight_original(&self, layer: usize, j: usize) -> &Weight {
        let pool_idx = *self.lsh2pool[layer]
            .get(&(j as u32))
            .expect("neuron index out of bounds");
        self.pool
            .pool_backup
            .get(pool_idx)
            .expect("could not get weight")
    }

    fn set_pool_backup(&mut self, layer: usize, j: usize) {
        let pool_idx = *self.lsh2pool[layer]
            .get(&(j as u32))
            .expect("neuron index out of bounds");
        self.pool.pool_backup[pool_idx] = self.pool.pool[pool_idx].clone();
    }

    fn get_biases(&self, layer: usize, idx: &[u32]) -> Vec<f32> {
        let lsh2bias = self.lsh2bias.get(layer).expect("Could not get bias layer");
        idx.iter()
            .map(|idx| *lsh2bias.get(idx).expect("Could not get bias"))
            .collect()
    }

    fn apply_layer(&self, i: usize, input: &[f32], last_layer: bool) -> Vec<Neuron> {
        let activ_fn = &self.activations[i];
        let mut idx_j: Vec<u32> = vec![];
        if last_layer {
            let output_dim = self.dimensions[self.dimensions.len() - 1];
            idx_j = (0_u32..output_dim as u32).collect()
        } else {
            let lsh = self.lsh_store[i].as_ref().unwrap();
            idx_j = lsh.query_bucket_ids(&input).unwrap();
        }

        let bias = self.get_biases(i, &idx_j);

        // index of the vectors in the pool
        let lsh2pool_i = &self.lsh2pool[i];
        let k: Vec<usize> = idx_j
            .iter()
            .map(|idx| *(lsh2pool_i.get(idx).unwrap()))
            .collect();
        let ps = self.pool.get(&k);

        ps.iter()
            .zip(bias)
            .zip(idx_j)
            .zip(k)
            .map(|(((&p, b), j), k)| {
                let j = j as usize;
                let z = aview1(&input).dot(p) + b;
                let a = activ_fn.activate(z);
                Neuron {
                    i,
                    j,
                    z,
                    a,
                    k,
                    delta: 0.,
                }
            })
            .collect()
    }

    pub fn forward(&self, x: &[f32]) -> (Vec<Vec<Neuron>>, Vec<Vec<f32>>) {
        let mut neur = Vec::with_capacity(self.n_layers);
        let mut inputs = Vec::with_capacity(self.n_layers);

        // first layer
        inputs.push(x.iter().copied().collect());
        let prev_neur = self.apply_layer(0, x, false);
        neur.push(prev_neur);

        for i in 1..self.n_layers - 1 {
            let prev_neur = neur.last().unwrap();
            let input = make_input_next_layer(prev_neur, self.dimensions[i]);
            let last_layer = i == (self.n_layers - 2);
            neur.push(self.apply_layer(i, &input, last_layer));
            inputs.push(input);
        }
        (neur, inputs)
    }

    pub fn backprop(&self, neur: &mut [Vec<Neuron>], y_true: &[u8]) -> f32 {
        // determine partial derivative and delta for output layer

        // iter only over the activations of the last layer
        // the loop is over all the perceptrons in one layer.
        // -2 because the of starting count from zero (-1)
        // and the input has no gradient update (-2)
        let n_activations_last_layer = neur[self.n_layers - 2].len();
        let mut loss = 0.;
        let mut delta;
        let (prev_layers, last_layer) = neur.split_at_mut(self.n_layers - 2);

        debug_assert_eq!(last_layer.len(), 1);
        for c in &mut last_layer[0] {
            let layer = self.n_layers - 2;
            debug_assert!(layer == c.i);

            delta = {
                let last_activation = &self.activations[self.activations.len() - 1];
                let y_true = y_true[c.j] as f32;
                let loss_fn = if self.loss == "mse" {
                    Loss::MSE(last_activation)
                } else {
                    Loss::NLL(last_activation)
                };
                loss += loss_fn.loss(y_true, c.a) / n_activations_last_layer as f32;
                loss_fn.delta(y_true, c.a)
            };

            c.delta = delta;

            // Track delta neurons:
            let mut prev_nodes = vec![];
            prev_nodes.push((delta, &*c as *const Neuron));
            let mut new_prev_nodes;

            // Per perceptron we traverse back all the layers (except the input)
            for layer in (0..self.n_layers - 2).rev() {
                for (prev_delta, prev_c) in &prev_nodes {
                    new_prev_nodes = vec![];
                    let prev_c = unsafe { &**prev_c };
                    // weights layer before
                    let w = self.get_weight(layer + 1, prev_c.j);
                    // activation layer before
                    let act = &self.activations[layer + 1];

                    for c in &mut prev_layers[layer] {
                        debug_assert!(layer == c.i);
                        delta = prev_delta * w[c.j] * act.prime(prev_c.z);
                        c.delta += delta;
                        new_prev_nodes.push((delta, &*c as *const Neuron));
                    }
                }
            }
        }
        loss
    }

    pub fn update_param(&mut self, input: &[f32], neur: &[Neuron]) {
        let lr = self.lr;
        let a = aview1(input);

        neur.iter().for_each(|neuron| {
            {
                let dw = &a * neuron.delta;
                let w = self.get_weight_mut(neuron.i, neuron.j as u32);
                azip!((w in w, &dw in &dw) *w = *w - lr * dw);
            }
            let b = self.get_bias_mut(neuron.i, neuron.j);
            *b = *b - lr * neuron.delta;
        });
    }

    pub fn rehash(&mut self) {
        for layer in 0..(self.n_layers - 1) {
            let shape = self.dimensions[layer + 1];

            // Take ownership
            let mut lsh = self
                .lsh_store
                .get_mut(layer)
                .expect("lsh index out of bounds")
                .take()
                .unwrap();

            (0..shape).for_each(|j| {
                let w = self.get_weight(layer, j);
                let w_original = self.get_weight_original(layer, j);
                let s = w.sum();
                let s_original = w_original.sum();

                // if they differ update lsh;
                if s != s_original {
                    lsh.update_by_idx(
                        j as u32,
                        &w.as_slice().unwrap(),
                        &w_original.as_slice().unwrap(),
                    );
                    self.set_pool_backup(layer, j);
                };
            });

            // restore lsh as it was.
            self.lsh_store[layer].replace(lsh);
        }
    }
}

#[derive(Debug)]
pub struct Neuron {
    // the ith layer in the network
    pub i: usize,
    // the jth perceptron in the layer (same as lsh idx)
    pub j: usize,
    // the kth layer in the memory pool
    k: usize,
    // wx + b of this perceptron
    pub z: f32,
    // activation of this perceptron
    pub a: f32,
    // input x (previous a)
    pub delta: f32,
}

fn make_input_next_layer(prev_neur: &[Neuron], layer_size: usize) -> Vec<f32> {
    // The previous layer had only a few of all possible activation.
    // create a new zero filled vector where only the activations are filled.

    let mut layer = vec![0.; layer_size];
    prev_neur.iter().for_each(|c| layer[c.j] = c.a);
    layer
}
