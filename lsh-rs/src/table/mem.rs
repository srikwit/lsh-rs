use super::general::{Bucket, HashTableError, HashTables};
use crate::hash::Hash;
use crate::utils::{all_eq, increase_capacity};
use crate::{DataPoint, DataPointSlice};
use fnv::FnvHashMap as HashMap;
use fnv::FnvHashSet as HashSet;
use serde::{Deserialize, Serialize};
use std::iter::FromIterator;

/// Indexible vector storage.
/// indexes will be stored in hashtables. The original vectors can be looked up in this data structure.
#[derive(Debug, Deserialize, Serialize)]
pub struct VecStore {
    pub map: Vec<DataPoint>,
}

impl VecStore {
    fn push(&mut self, d: DataPoint) -> u32 {
        self.map.push(d);
        (self.map.len() - 1) as u32
    }

    fn position(&self, d: &DataPointSlice) -> Option<u32> {
        self.map.iter().position(|x| all_eq(x, d)).map(|x| x as u32)
    }

    fn get(&self, idx: u32) -> &DataPoint {
        &self.map[idx as usize]
    }

    fn increase_storage(&mut self, size: usize) {
        increase_capacity(size, &mut self.map);
    }
}

/// In memory storage of hashed vectors/ indexes.
#[derive(Deserialize, Serialize)]
pub struct MemoryTable {
    hash_tables: Vec<HashMap<Hash, Bucket>>,
    n_hash_tables: usize,
    pub vec_store: VecStore,
    only_index_storage: bool,
    counter: u32,
}

impl MemoryTable {
    pub fn new(n_hash_tables: usize, only_index_storage: bool) -> Self {
        // TODO: Check the average number of vectors in the buckets.
        // this way the capacity can be approximated by the number of DataPoints that will
        // be stored.
        let hash_tables = vec![HashMap::default(); n_hash_tables];
        let vector_store = VecStore { map: vec![] };
        MemoryTable {
            hash_tables,
            n_hash_tables,
            vec_store: vector_store,
            only_index_storage,
            counter: 0,
        }
    }
}

impl HashTables for MemoryTable {
    fn put(
        &mut self,
        hash: Hash,
        d: &DataPointSlice,
        hash_table: usize,
    ) -> Result<u32, HashTableError> {
        let tbl = &mut self.hash_tables[hash_table];

        // Store hash and id/idx
        let idx = self.counter;
        let bucket = tbl.entry(hash).or_insert_with(|| HashSet::default());
        bucket.insert(idx);

        // There are N hash_tables per unique vector. So we only store
        // the unique v hash_table 0 and increment the counter (the id)
        // after we've update the last (N) hash_table.
        if (hash_table == 0) && (!self.only_index_storage) {
            self.vec_store.push(d.to_vec());
        } else if hash_table == self.n_hash_tables - 1 {
            self.counter += 1
        }
        Ok(idx)
    }

    /// Expensive operation we need to do a linear search over all datapoints
    fn delete(
        &mut self,
        hash: Hash,
        d: &DataPointSlice,
        hash_table: usize,
    ) -> Result<(), HashTableError> {
        // First find the data point in the VecStore
        let idx = match self.vec_store.position(d) {
            None => return Ok(()),
            Some(idx) => idx,
        };
        // Note: data point remains in VecStore as shrinking the vector would mean we need to
        // re-hash all datapoints.

        // Then remove idx from hash tables
        let tbl = &mut self.hash_tables[hash_table];
        let bucket = tbl.get_mut(&hash);
        match bucket {
            None => return Err(HashTableError::NotFound),
            Some(bucket) => {
                bucket.remove(&idx);
                Ok(())
            }
        }
    }

    /// Query the whole bucket
    fn query_bucket(&self, hash: &Hash, hash_table: usize) -> Result<Bucket, HashTableError> {
        let tbl = &self.hash_tables[hash_table];
        match tbl.get(hash) {
            None => Err(HashTableError::NotFound),
            Some(bucket) => Ok(bucket.clone()),
        }
    }

    fn idx_to_datapoint(&self, idx: u32) -> Result<&DataPoint, HashTableError> {
        Ok(self.vec_store.get(idx))
    }

    fn increase_storage(&mut self, size: usize) {
        increase_capacity(size, &mut self.hash_tables);
        self.vec_store.increase_storage(size);
    }

    fn describe(&self) {
        let mut lengths = vec![];
        let mut max_len = 0;
        let mut min_len = 1000000;
        let mut set: HashSet<i32> = HashSet::default();
        for map in self.hash_tables.iter() {
            for (k, v) in map.iter() {
                let len = v.len();
                let hash_values: HashSet<i32> = HashSet::from_iter(k.iter().copied());
                set = set.union(&hash_values).copied().collect();
                lengths.push(len);
                if len > max_len {
                    max_len = len
                }
                if len < min_len {
                    min_len = len
                }
            }
        }

        println!(
            "Bucket lengths: max: {}, min: {}, avg: {}, hash value projections: {:?}",
            max_len,
            min_len,
            lengths.iter().sum::<usize>() as f32 / lengths.len() as f32,
            set
        )
    }
}

impl std::fmt::Debug for MemoryTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "hash_tables:\nhash, \t buckets\n")?;
        for ht in self.hash_tables.iter() {
            write!(f, "{:?}\n", ht)?;
        }
        Ok(())
    }
}