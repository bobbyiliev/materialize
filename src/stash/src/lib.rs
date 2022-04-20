// Copyright Materialize, Inc. and contributors. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Durable metadata storage.

use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fmt::Debug;
use std::hash::Hash;
use std::iter;
use std::iter::once;
use std::marker::PhantomData;

use mz_ore::soft_assert;
use timely::progress::frontier::AntichainRef;
use timely::progress::Antichain;
use timely::PartialOrder;

use mz_ore::collections::CollectionExt;
use mz_persist_types::Codec;

mod postgres;
mod sqlite;

pub use crate::postgres::Postgres;
pub use crate::sqlite::Sqlite;

pub type Diff = i64;
pub type Timestamp = i64;
pub type Id = i64;

/// A durable metadata store.
///
/// A stash manages any number of named [`StashCollection`]s.
///
/// A stash is designed to store only a small quantity of data. Think megabytes,
/// not gigabytes.
///
/// The API of a stash intentionally mimics the API of a [STORAGE] collection.
/// You can think of stash as a stable but very low performance STORAGE
/// collection. When the STORAGE layer is stable enough to serve as a source of
/// truth, the intent is to swap all stashes for STORAGE collections.
///
/// [STORAGE]: https://github.com/MaterializeInc/materialize/blob/main/doc/developer/platform/architecture-db.md#STORAGE
pub trait Stash {
    /// Loads or creates the named collection.
    ///
    /// If the collection with the specified name does not yet exist, it is
    /// created with no entries, a zero since frontier, and a zero upper
    /// frontier. Otherwise the existing durable state is loaded.
    ///
    /// It is the callers responsibility to keep `K` and `V` fixed for a given
    /// collection in a given stash for the lifetime of the stash.
    ///
    /// It is valid to construct multiple handles to the same named collection
    /// and use them simultaneously.
    fn collection<K, V>(&mut self, name: &str) -> Result<StashCollection<K, V>, StashError>
    where
        K: Codec + Ord,
        V: Codec + Ord;

    /// Iterates over all entries in the stash.
    ///
    /// Entries are iterated in `(key, value, time)` order and are guaranteed
    /// to be consolidated.
    ///
    /// Each entry's time is guaranteed to be greater than or equal to the since
    /// frontier. The time may also be greater than the upper frontier,
    /// indicating data that has not yet been made definite.
    fn iter<K, V>(
        &mut self,
        collection: StashCollection<K, V>,
    ) -> Result<Vec<((K, V), Timestamp, Diff)>, StashError>
    where
        K: Codec + Ord,
        V: Codec + Ord;

    /// Iterates over entries in the stash for the given key.
    ///
    /// Entries are iterated in `(value, timestamp)` order and are guaranteed
    /// to be consolidated.
    ///
    /// Each entry's time is guaranteed to be greater than or equal to the since
    /// frontier. The time may also be greater than the upper frontier,
    /// indicating data that has not yet been made definite.
    fn iter_key<K, V>(
        &mut self,
        collection: StashCollection<K, V>,
        key: &K,
    ) -> Result<Vec<(V, Timestamp, Diff)>, StashError>
    where
        K: Codec + Ord,
        V: Codec + Ord;

    /// Returns the most recent timestamp at which sealed entries can be read.
    fn peek_timestamp<K, V>(
        &mut self,
        collection: StashCollection<K, V>,
    ) -> Result<Timestamp, StashError>
    where
        K: Codec + Ord,
        V: Codec + Ord,
    {
        let since = self.since(collection)?;
        let upper = self.upper(collection)?;
        if PartialOrder::less_equal(&upper, &since) {
            return Err(StashError {
                inner: InternalStashError::PeekSinceUpper(format!(
                    "collection since {} is not less than upper {}",
                    AntichainFormatter(&since),
                    AntichainFormatter(&upper)
                )),
            });
        }
        match upper.as_option() {
            Some(ts) => match ts.checked_sub(1) {
                Some(ts) => Ok(ts),
                None => Err("could not determine peek timestamp".into()),
            },
            None => Ok(Timestamp::MAX),
        }
    }

    /// Returns the current value of sealed entries.
    ///
    /// Entries are iterated in `(key, value)` order and are guaranteed to be
    /// consolidated.
    ///
    /// Sealed entries are those with timestamps less than the collection's upper
    /// frontier.
    fn peek<K, V>(
        &mut self,
        collection: StashCollection<K, V>,
    ) -> Result<Vec<(K, V, Diff)>, StashError>
    where
        K: Codec + Ord,
        V: Codec + Ord,
    {
        let timestamp = self.peek_timestamp(collection)?;
        let mut rows: Vec<_> = self
            .iter(collection)?
            .into_iter()
            .filter_map(|((k, v), data_ts, diff)| {
                if data_ts.less_equal(&timestamp) {
                    Some((k, v, diff))
                } else {
                    None
                }
            })
            .collect();
        differential_dataflow::consolidation::consolidate_updates(&mut rows);
        Ok(rows)
    }

    /// Returns the current k,v pairs of sealed entries, erroring if there is more
    /// than one entry for a given key or the multiplicity is not 1 for each key.
    ///
    /// Sealed entries are those with timestamps less than the collection's upper
    /// frontier.
    fn peek_one<K, V>(
        &mut self,
        collection: StashCollection<K, V>,
    ) -> Result<BTreeMap<K, V>, StashError>
    where
        K: Codec + Ord + std::hash::Hash,
        V: Codec + Ord,
    {
        let rows = self.peek(collection)?;
        let mut res = BTreeMap::new();
        for (k, v, diff) in rows {
            if diff != 1 {
                return Err("unexpected peek multiplicity".into());
            }
            if res.insert(k, v).is_some() {
                return Err("duplicate peek keys".into());
            }
        }
        Ok(res)
    }

    /// Returns the current sealed value for the given key, erroring if there is
    /// more than one entry for the key or its multiplicity is not 1.
    ///
    /// Sealed entries are those with timestamps less than the collection's upper
    /// frontier.
    fn peek_key_one<K, V>(
        &mut self,
        collection: StashCollection<K, V>,
        key: &K,
    ) -> Result<Option<V>, StashError>
    where
        K: Codec + Ord,
        V: Codec + Ord,
    {
        let timestamp = self.peek_timestamp(collection)?;
        let mut rows: Vec<_> = self
            .iter_key(collection, key)?
            .into_iter()
            .filter_map(|(v, data_ts, diff)| {
                if data_ts.less_equal(&timestamp) {
                    Some((v, diff))
                } else {
                    None
                }
            })
            .collect();
        differential_dataflow::consolidation::consolidate(&mut rows);
        let v = match rows.len() {
            1 => {
                let (v, diff) = rows.into_element();
                match diff {
                    1 => Some(v),
                    0 => None,
                    _ => return Err("multiple values unexpected".into()),
                }
            }
            0 => None,
            _ => return Err("multiple values unexpected".into()),
        };
        Ok(v)
    }

    /// Adds a single entry to the arrangement.
    ///
    /// The entry's time must be greater than or equal to the upper frontier.
    ///
    /// If this method returns `Ok`, the entry has been made durable.
    fn update<K: Codec, V: Codec>(
        &mut self,
        collection: StashCollection<K, V>,
        data: (K, V),
        time: Timestamp,
        diff: Diff,
    ) -> Result<(), StashError> {
        self.update_many(collection, iter::once((data, time, diff)))
    }

    /// Atomically adds multiple entries to the arrangement.
    ///
    /// Each entry's time must be greater than or equal to the upper frontier.
    ///
    /// If this method returns `Ok`, the entries have been made durable.
    fn update_many<K: Codec, V: Codec, I>(
        &mut self,
        collection: StashCollection<K, V>,
        entries: I,
    ) -> Result<(), StashError>
    where
        I: IntoIterator<Item = ((K, V), Timestamp, Diff)>;

    /// Advances the upper frontier to the specified value.
    ///
    /// The provided `upper` must be greater than or equal to the current upper
    /// frontier.
    ///
    /// Intuitively, this method declares that all times less than `upper` are
    /// definite.
    fn seal<K, V>(
        &mut self,
        collection: StashCollection<K, V>,
        upper: AntichainRef<Timestamp>,
    ) -> Result<(), StashError>;

    /// Performs multiple seals at once, potentially in a more performant way than
    /// performing the individual seals one by one.
    ///
    /// See [Stash::seal]
    fn seal_batch<K, V>(
        &mut self,
        seals: &[(StashCollection<K, V>, Antichain<Timestamp>)],
    ) -> Result<(), StashError> {
        for (id, upper) in seals {
            self.seal(*id, upper.borrow())?;
        }
        Ok(())
    }

    /// Advances the since frontier to the specified value.
    ///
    /// The provided `since` must be greater than or equal to the current since
    /// frontier but less than or equal to the current upper frontier.
    ///
    /// Intuitively, this method performs logical compaction. Existing entries
    /// whose time is less than `since` are fast-forwarded to `since`.
    fn compact<K, V>(
        &mut self,
        collection: StashCollection<K, V>,
        since: AntichainRef<Timestamp>,
    ) -> Result<(), StashError>;

    /// Performs multiple compactions at once, potentially in a more performant way than
    /// performing the individual compactions one by one.
    ///
    /// See [Stash::compact]
    fn compact_batch<K, V>(
        &mut self,
        compactions: &[(StashCollection<K, V>, Antichain<Timestamp>)],
    ) -> Result<(), StashError> {
        for (id, since) in compactions {
            self.compact(*id, since.borrow())?;
        }
        Ok(())
    }

    /// Consolidates entries less than the since frontier.
    ///
    /// Intuitively, this method performs physical compaction. Existing
    /// key–value pairs whose time is less than the since frontier are
    /// consolidated together when possible.
    fn consolidate<K, V>(&mut self, collection: StashCollection<K, V>) -> Result<(), StashError>;

    /// Performs multiple consolidations at once, potentially in a more performant way than
    /// performing the individual consolidations one by one.
    ///
    /// See [Stash::consolidate]
    fn consolidate_batch<K, V>(
        &mut self,
        collections: &[StashCollection<K, V>],
    ) -> Result<(), StashError> {
        for collection in collections {
            self.consolidate(*collection)?;
        }
        Ok(())
    }

    /// Reports the current since frontier.
    fn since<K, V>(
        &mut self,
        collection: StashCollection<K, V>,
    ) -> Result<Antichain<Timestamp>, StashError>;

    /// Reports the current upper frontier.
    fn upper<K, V>(
        &mut self,
        collection: StashCollection<K, V>,
    ) -> Result<Antichain<Timestamp>, StashError>;
}

/// `StashCollection` is like a differential dataflow [`Collection`], but the
/// state of the collection is durable.
///
/// A `StashCollection` stores `(key, value, timestamp, diff)` entries. The key
/// and value types are chosen by the caller; they must implement [`Ord`] and
/// they must be serializable to and deserializable from bytes via the [`Codec`]
/// trait. The timestamp and diff types are fixed to `i64`.
///
/// A `StashCollection` maintains a since frontier and an upper frontier, as
/// described in the [correctness vocabulary document]. To advance the since
/// frontier, call [`compact`]. To advance the upper frontier, call [`seal`]. To
/// physically compact data beneath the since frontier, call [`consolidate`].
///
/// [`compact`]: Stash::compact
/// [`consolidate`]: Stash::consolidate
/// [`seal`]: Stash::seal
/// [correctness vocabulary document]: https://github.com/MaterializeInc/materialize/blob/main/doc/developer/design/20210831_correctness.md
/// [`Collection`]: differential_dataflow::collection::Collection
pub struct StashCollection<K, V> {
    id: Id,
    _kv: PhantomData<(K, V)>,
}

impl<K, V> Clone for StashCollection<K, V> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            _kv: PhantomData,
        }
    }
}

impl<K, V> Copy for StashCollection<K, V> {}

struct AntichainFormatter<'a, T>(&'a [T]);

impl<T> fmt::Display for AntichainFormatter<'_, T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("{")?;
        for (i, element) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            element.fmt(f)?;
        }
        f.write_str("}")
    }
}

impl<'a, T> From<&'a Antichain<T>> for AntichainFormatter<'a, T> {
    fn from(antichain: &Antichain<T>) -> AntichainFormatter<T> {
        AntichainFormatter(antichain.elements())
    }
}

/// An error that can occur while interacting with a [`Stash`].
///
/// Stash errors are deliberately opaque. They generally indicate unrecoverable
/// conditions, like running out of disk space.
#[derive(Debug)]
pub struct StashError {
    // Internal to avoid leaking implementation details about SQLite.
    inner: InternalStashError,
}

impl StashError {
    // Returns whether the error is unrecoverable (retrying will never succeed).
    pub fn is_unrecoverable(&self) -> bool {
        matches!(self.inner, InternalStashError::Fence(_))
    }
}

#[derive(Debug)]
enum InternalStashError {
    Sqlite(rusqlite::Error),
    Postgres(::postgres::Error),
    Fence(String),
    PeekSinceUpper(String),
    Other(String),
}

impl fmt::Display for StashError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("stash error: ")?;
        match &self.inner {
            InternalStashError::Sqlite(e) => std::fmt::Display::fmt(&e, f),
            InternalStashError::Postgres(e) => std::fmt::Display::fmt(&e, f),
            InternalStashError::Fence(e) => f.write_str(&e),
            InternalStashError::PeekSinceUpper(e) => f.write_str(&e),
            InternalStashError::Other(e) => f.write_str(&e),
        }
    }
}

impl Error for StashError {}

impl From<InternalStashError> for StashError {
    fn from(inner: InternalStashError) -> StashError {
        StashError { inner }
    }
}

impl From<String> for StashError {
    fn from(e: String) -> StashError {
        StashError {
            inner: InternalStashError::Other(e),
        }
    }
}

impl From<&str> for StashError {
    fn from(e: &str) -> StashError {
        StashError {
            inner: InternalStashError::Other(e.into()),
        }
    }
}

pub trait Append: Stash {
    fn append<I: IntoIterator<Item = AppendBatch>>(&mut self, batches: I)
        -> Result<(), StashError>;
}

#[derive(Clone, Debug)]
pub struct AppendBatch {
    pub collection_id: Id,
    pub lower: Antichain<Timestamp>,
    pub upper: Antichain<Timestamp>,
    pub timestamp: Timestamp,
    pub entries: Vec<((Vec<u8>, Vec<u8>), Timestamp, Diff)>,
}

impl<K, V> StashCollection<K, V>
where
    K: Codec + Ord,
    V: Codec + Ord,
{
    /// Create a new AppendBatch for this collection from its current upper.
    pub fn make_batch<S: Stash>(&self, stash: &mut S) -> Result<AppendBatch, StashError> {
        let lower = stash.upper(*self)?;
        let timestamp: Timestamp = match lower.elements() {
            [ts] => *ts,
            _ => return Err("cannot determine batch timestamp".into()),
        };
        let upper = match timestamp.checked_add(1) {
            Some(ts) => Antichain::from_elem(ts),
            None => return Err("cannot determine new upper".into()),
        };
        Ok(AppendBatch {
            collection_id: self.id,
            lower,
            upper,
            timestamp,
            entries: Vec::new(),
        })
    }

    pub fn append_to_batch(&self, batch: &mut AppendBatch, key: &K, value: &V, diff: Diff) {
        let mut key_buf = vec![];
        let mut value_buf = vec![];
        key.encode(&mut key_buf);
        value.encode(&mut value_buf);
        batch
            .entries
            .push(((key_buf, value_buf), batch.timestamp, diff));
    }
}

impl<K, V> From<Id> for StashCollection<K, V> {
    fn from(id: Id) -> Self {
        Self {
            id,
            _kv: PhantomData,
        }
    }
}

/// A helper struct to prevent mistyping of a [`StashCollection`]'s name and
/// k,v types.
pub struct TypedCollection<K, V> {
    name: &'static str,
    typ: PhantomData<(K, V)>,
}

impl<K, V> TypedCollection<K, V> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            typ: PhantomData,
        }
    }
}

impl<K, V> TypedCollection<K, V>
where
    K: Codec + Ord,
    V: Codec + Ord,
{
    pub fn get(&self, stash: &mut impl Stash) -> Result<StashCollection<K, V>, StashError> {
        stash.collection(self.name)
    }

    pub fn peek_one<S>(&self, stash: &mut S) -> Result<BTreeMap<K, V>, StashError>
    where
        S: Stash,
        K: Hash,
    {
        let collection = self.get(stash)?;
        stash.peek_one(collection)
    }

    pub fn peek_key_one(&self, stash: &mut impl Stash, key: &K) -> Result<Option<V>, StashError> {
        let collection = self.get(stash)?;
        stash.peek_key_one(collection, key)
    }

    /// Sets the given k,v pair.
    pub fn upsert_key<S>(&self, stash: &mut S, key: &K, value: &V) -> Result<(), StashError>
    where
        S: Append,
    {
        let collection = self.get(stash)?;
        let mut batch = collection.make_batch(stash)?;
        let prev = match stash.peek_key_one(collection, key) {
            Ok(prev) => prev,
            Err(err) => match err.inner {
                InternalStashError::PeekSinceUpper(_) => {
                    // If the upper isn't > since, bump the upper and try again to find a sealed
                    // entry. Do this by appending the empty batch which will advance the upper.
                    stash.append(once(batch))?;
                    batch = collection.make_batch(stash)?;
                    stash.peek_key_one(collection, key)?
                }
                _ => return Err(err),
            },
        };
        if let Some(prev) = &prev {
            collection.append_to_batch(&mut batch, &key, &prev, -1);
        }
        collection.append_to_batch(&mut batch, &key, &value, 1);
        stash.append(once(batch))?;
        Ok(())
    }

    /// Sets the given key value pairs, removing existing entries match any key.
    pub fn upsert<S, I>(&self, stash: &mut S, entries: I) -> Result<(), StashError>
    where
        S: Append,
        I: IntoIterator<Item = (K, V)>,
        K: Hash,
    {
        let collection = self.get(stash)?;
        let mut batch = collection.make_batch(stash)?;
        let prev = stash.peek_one(collection)?;
        for (k, v) in entries {
            if let Some(prev_v) = prev.get(&k) {
                collection.append_to_batch(&mut batch, &k, &prev_v, -1);
            }
            collection.append_to_batch(&mut batch, &k, &v, 1);
        }
        stash.append(once(batch))?;
        Ok(())
    }
}

/// TableTransaction emulates some features of a typical SQL transaction over
/// table for a [`StashCollection`].
///
/// It supports:
/// - auto increment primary keys
/// - uniqueness constraints
/// - transactional reads and writes (including read-your-writes before commit)
///
/// `K` is the primary key type. Multiple entries with the same key are disallowed.
/// `V` is the an arbitrary value type.
/// `I` is the type of an autoincrementing number (like `i64`).
///
/// To finalize, add the results of [`TableTransaction::pending()`] to an
/// [`AppendBatch`].
pub struct TableTransaction<K, V, I> {
    initial: BTreeMap<K, V>,
    // The desired state of keys after commit. `None` means the value will be
    // deleted.
    pending: BTreeMap<K, Option<V>>,
    next_id: Option<I>,
    uniqueness_violation: fn(a: &V, b: &V) -> bool,
}

impl<K, V, I> TableTransaction<K, V, I>
where
    K: Ord + Eq + Hash + Clone,
    V: Ord + Clone,
    I: Copy + Ord + Default + num::Num + num::CheckedAdd,
{
    /// Create a new TableTransaction with initial data. `numeric_identity` is a
    /// function to extract an ID from the initial keys (or `None` to disable auto
    /// incrementing). `uniqueness_violation` is a function whether there is a
    /// uniqueness violation among two values.
    pub fn new(
        initial: BTreeMap<K, V>,
        numeric_identity: Option<fn(k: &K) -> I>,
        uniqueness_violation: fn(a: &V, b: &V) -> bool,
    ) -> Self {
        let next_id = numeric_identity.map(|f| {
            initial
                .keys()
                .map(f)
                .max()
                .unwrap_or_default()
                .checked_add(&I::one())
                .expect("ids exhausted")
        });
        Self {
            initial,
            pending: BTreeMap::new(),
            next_id,
            uniqueness_violation,
        }
    }

    /// Consumes and returns the pending changes and their diffs. `Diff` is
    /// guaranteed to be 1 or -1.
    pub fn pending(self) -> Vec<(K, V, Diff)> {
        soft_assert!(self.verify().is_ok());
        // Pending describes the desired final state for some keys. K,V pairs should be
        // retracted if they already exist and were deleted or are being updated.
        self.pending
            .into_iter()
            .map(|(k, v)| match self.initial.get(&k) {
                Some(initial_v) => {
                    let mut diffs = vec![(k.clone(), initial_v.clone(), -1)];
                    if let Some(v) = v {
                        diffs.push((k, v, 1));
                    }
                    diffs
                }
                None => {
                    if let Some(v) = v {
                        vec![(k, v, 1)]
                    } else {
                        vec![]
                    }
                }
            })
            .flatten()
            .collect()
    }

    fn verify(&self) -> Result<(), StashError> {
        // Compare each value to each other value and ensure they are unique.
        let items = self.items();
        for (i, vi) in items.values().enumerate() {
            for (j, vj) in items.values().enumerate() {
                if i != j && (self.uniqueness_violation)(vi, vj) {
                    return Err("uniqueness violation".into());
                }
            }
        }
        Ok(())
    }

    /// Iterates over the items viewable in the current transaction in arbitrary
    /// order.
    pub fn for_values<F: FnMut(&K, &V)>(&self, mut f: F) {
        let mut seen = HashSet::new();
        for (k, v) in self.pending.iter() {
            seen.insert(k);
            // Deleted items don't exist so shouldn't be visited, but still suppress
            // visiting the key later.
            if let Some(v) = v {
                f(k, v);
            }
        }
        for (k, v) in self.initial.iter() {
            // Add on initial items that don't have updates.
            if !seen.contains(k) {
                f(k, v);
            }
        }
    }

    /// Returns the items viewable in the current transaction.
    pub fn items(&self) -> BTreeMap<K, V> {
        let mut items = BTreeMap::new();
        self.for_values(|k, v| {
            items.insert(k.clone(), v.clone());
        });
        items
    }

    /// Iterates over the items viewable in the current transaction, and provides a
    /// Vec where additional pending items can be inserted, which will be appended
    /// to current pending items. Does not verify unqiueness.
    fn for_values_mut<F: FnMut(&mut BTreeMap<K, Option<V>>, &K, &V)>(&mut self, mut f: F) {
        let mut pending = BTreeMap::new();
        self.for_values(|k, v| f(&mut pending, k, v));
        self.pending.extend(pending);
    }

    /// Inserts a new k,v pair. `key_from_id` is a function that returns a new `K`
    /// provided a new autoincremented `I` (`key_from_id` will be passed `None`
    /// if auto increment was disabled above, but it must still return the new
    /// key). The new id is guaranteed to be different from all current and pending
    /// ids (but not all ids ever).
    ///
    /// Returns an error if the uniqueness check failed or the key already exists.
    pub fn insert<F: FnOnce(Option<I>) -> K>(
        &mut self,
        key_from_id: F,
        v: V,
    ) -> Result<Option<I>, ()> {
        let id = self.next_id;
        let next_id = self
            .next_id
            .map(|id| id.checked_add(&I::one()).expect("ids exhausted"));
        let k = key_from_id(id);
        let mut violation = false;
        self.for_values(|for_k, for_v| {
            if &k == for_k || (self.uniqueness_violation)(for_v, &v) {
                violation = true;
            }
        });
        if violation {
            return Err(());
        }
        self.pending.insert(k, Some(v));
        soft_assert!(self.verify().is_ok());
        self.next_id = next_id;
        Ok(id)
    }

    /// Updates k, v pairs. `f` is a function that can return `Some(V)` if the
    /// value should be updated, otherwise `None`. Returns the number of changed
    /// entries.
    ///
    /// Returns an error if the uniqueness check failed.
    pub fn update<F: Fn(&K, &V) -> Option<V>>(&mut self, f: F) -> Result<Diff, StashError> {
        let mut changed = 0;
        // Keep a copy of pending in case of uniqueness violation.
        let pending = self.pending.clone();
        self.for_values_mut(|p, k, v| {
            if let Some(next) = f(k, v) {
                changed += 1;
                p.insert(k.clone(), Some(next));
            }
        });
        // Check for uniqueness violation.
        if let Err(err) = self.verify() {
            self.pending = pending;
            Err(err)
        } else {
            Ok(changed)
        }
    }

    /// Deletes items for which `f` returns true. Returns the number of deleted
    /// items.
    pub fn delete<F: Fn(&K, &V) -> bool>(&mut self, f: F) -> Diff {
        let mut changed = 0;
        self.for_values_mut(|p, k, v| {
            if f(k, v) {
                changed += 1;
                p.insert(k.clone(), None);
            }
        });
        soft_assert!(self.verify().is_ok());
        changed
    }
}
