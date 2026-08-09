use std::fmt::Formatter;

use gix_hash::ObjectId;
use gix_object::bstr::BString;

use crate::{
    store_impl::{file, file::Transaction},
    transaction::RefEdit,
};

/// How to handle packed refs during a transaction
#[derive(Default)]
pub enum PackedRefs<'a> {
    /// Only propagate deletions of references. This is the default.
    /// This means deleted references are removed from disk if they are loose and from the packed-refs file if they are present.
    #[default]
    DeletionsOnly,
    /// Propagate deletions as well as updates to references which are peeled and contain an object id.
    ///
    /// This means deleted references are removed from disk if they are loose and from the packed-refs file if they are present,
    /// while updates are also written into the loose file as well as into packed-refs, potentially creating an entry.
    DeletionsAndNonSymbolicUpdates(Box<dyn gix_object::Find + 'a>),
    /// Propagate deletions as well as updates to references which are peeled and contain an object id. Furthermore delete the
    /// reference which is originally updated if it exists. If it doesn't, the new value will be written into the packed ref right away.
    /// Note that this doesn't affect symbolic references at all, which can't be placed into packed refs.
    ///
    /// Thus, this is similar to `DeletionsAndNonSymbolicUpdates`, but removes the loose reference after the update, leaving only their copy
    /// in `packed-refs`.
    DeletionsAndNonSymbolicUpdatesRemoveLooseSourceReference(Box<dyn gix_object::Find + 'a>),
}

#[derive(Debug)]
pub(in crate::store_impl::file) struct Edit {
    update: RefEdit,
    lock: Option<gix_lock::Marker>,
    /// Set if this update is coming from a symbolic reference and used to make it appear like it is the one that is handled,
    /// instead of the referent reference.
    parent_index: Option<usize>,
    /// The previous OID to put into the reflog instead of deriving it from the stored-target constraint.
    reflog_previous_oid: Option<ObjectId>,
    /// The previous and new peeled OIDs to put into the reflog for a symbolic-target update.
    symbolic_reflog_oids: Option<(ObjectId, ObjectId)>,
    /// Write the reflog even when its previous and new OIDs are equal.
    force_reflog_update: bool,
}

impl Edit {
    fn name(&self) -> BString {
        self.update.name.0.clone()
    }
}

impl std::borrow::Borrow<RefEdit> for Edit {
    fn borrow(&self) -> &RefEdit {
        &self.update
    }
}

impl std::borrow::BorrowMut<RefEdit> for Edit {
    fn borrow_mut(&mut self) -> &mut RefEdit {
        &mut self.update
    }
}

/// Edits
impl file::Store {
    /// Open a transaction with the given `edits`, and determine how to fail if a `lock` cannot be obtained.
    /// A snapshot of packed references will be obtained automatically if needed to fulfill this transaction
    /// and will be provided as result of a successful transaction. Note that upon transaction failure, packed-refs
    /// will never have been altered.
    ///
    /// The transaction inherits the parent namespace.
    pub fn transaction(&self) -> Transaction<'_, '_> {
        Transaction {
            store: self,
            packed_transaction: None,
            updates: None,
            loose_ref_guards: Vec::new(),
            packed_refs: PackedRefs::default(),
        }
    }
}

impl<'p> Transaction<'_, 'p> {
    /// Configure the way packed refs are handled during the transaction
    pub fn packed_refs(mut self, packed_refs: PackedRefs<'p>) -> Self {
        self.packed_refs = packed_refs;
        self
    }

    /// Use `previous_oid` as the previous object ID in the reflog entry for the prepared object update named `name`.
    ///
    /// This allows the reflog value to remain independent of the stored-target constraint used during
    /// [`prepare()`][Transaction::prepare()]. It is useful when replacing a symbolic reference without dereferencing it:
    /// the constraint can compare the symbolic target while the reflog records the symbolic reference's peeled object ID.
    pub fn with_reflog_previous_oid(
        mut self,
        name: &crate::FullNameRef,
        previous_oid: ObjectId,
    ) -> Result<Self, with_reflog_previous_oid::Error> {
        let edit = self.prepared_object_update_mut(name)?;
        edit.reflog_previous_oid = Some(previous_oid);
        Ok(self)
    }

    /// Write a reflog entry for the prepared object update named `name` even if its previous and new OIDs are equal.
    ///
    /// This is useful when the stored representation changes, such as replacing a symbolic reference with a direct
    /// reference to the same object. The ref update remains subject to the constraint supplied to
    /// [`prepare()`][Transaction::prepare()].
    pub fn force_reflog_update(mut self, name: &crate::FullNameRef) -> Result<Self, with_reflog_previous_oid::Error> {
        let edit = self.prepared_object_update_mut(name)?;
        edit.force_reflog_update = true;
        Ok(self)
    }

    /// Write the reflog for the prepared symbolic update named `name` with its peeled object IDs.
    ///
    /// Symbolic targets carry no object IDs themselves, so callers that have peeled the previous and new targets may
    /// provide both values without weakening the stored-target constraint supplied to [`prepare()`][Transaction::prepare()].
    /// Unlike object updates, an explicit symbolic reflog update is written even when both targets peel to the same object.
    pub fn with_symbolic_reflog(
        mut self,
        name: &crate::FullNameRef,
        previous_oid: ObjectId,
        new_oid: ObjectId,
    ) -> Result<Self, with_reflog_previous_oid::Error> {
        let edit = self.prepared_symbolic_update_mut(name)?;
        edit.symbolic_reflog_oids = Some((previous_oid, new_oid));
        Ok(self)
    }

    /// Ensure the prepared edit named `name` holds the lock for its loose reference path.
    ///
    /// A transaction that already holds the named lock is unchanged. This is useful when a no-op update released its
    /// ordinary update lock, but its caller must still exclude a concurrent loose reference update before revalidating
    /// an external precondition.
    pub fn ensure_ref_lock(
        mut self,
        name: &crate::FullNameRef,
        lock_fail_mode: gix_lock::acquire::Fail,
    ) -> Result<Self, ensure_ref_lock::Error> {
        let (base, relative_path) = self.store.reference_path_with_base(name);
        let updates = self.updates.as_mut().ok_or(ensure_ref_lock::Error::Unprepared)?;
        let mut matches = updates.iter_mut().filter(|edit| edit.update.name.as_ref() == name);
        let edit = matches
            .next()
            .ok_or_else(|| ensure_ref_lock::Error::MissingEdit { name: name.to_owned() })?;
        if matches.next().is_some() {
            return Err(ensure_ref_lock::Error::AmbiguousEdit { name: name.to_owned() });
        }
        if edit.lock.is_none() {
            let marker = gix_lock::Marker::acquire_to_hold_resource(
                base.join(relative_path.as_ref()),
                lock_fail_mode,
                Some(base.into_owned()),
            )
            .map_err(|source| ensure_ref_lock::Error::LockAcquire {
                source,
                name: name.to_owned(),
            })?;
            if matches!(edit.update.change, crate::transaction::Change::Update { .. }) {
                self.loose_ref_guards.push(marker);
            } else {
                edit.lock = Some(marker);
            }
        }
        Ok(self)
    }

    fn prepared_object_update_mut(
        &mut self,
        name: &crate::FullNameRef,
    ) -> Result<&mut Edit, with_reflog_previous_oid::Error> {
        let edit = self.prepared_update_mut(name)?;
        if !matches!(
            edit.update.change,
            crate::transaction::Change::Update {
                new: crate::Target::Object(_),
                ..
            }
        ) {
            return Err(with_reflog_previous_oid::Error::NotObjectUpdate { name: name.to_owned() });
        }
        Ok(edit)
    }

    fn prepared_symbolic_update_mut(
        &mut self,
        name: &crate::FullNameRef,
    ) -> Result<&mut Edit, with_reflog_previous_oid::Error> {
        let edit = self.prepared_update_mut(name)?;
        if !matches!(
            edit.update.change,
            crate::transaction::Change::Update {
                new: crate::Target::Symbolic(_),
                ..
            }
        ) {
            return Err(with_reflog_previous_oid::Error::NotSymbolicUpdate { name: name.to_owned() });
        }
        Ok(edit)
    }

    fn prepared_update_mut(&mut self, name: &crate::FullNameRef) -> Result<&mut Edit, with_reflog_previous_oid::Error> {
        let updates = self
            .updates
            .as_mut()
            .ok_or(with_reflog_previous_oid::Error::Unprepared)?;
        let mut matches = updates.iter_mut().filter(|edit| edit.update.name.as_ref() == name);
        let edit = matches
            .next()
            .ok_or_else(|| with_reflog_previous_oid::Error::MissingEdit { name: name.to_owned() })?;
        if matches.next().is_some() {
            return Err(with_reflog_previous_oid::Error::AmbiguousEdit { name: name.to_owned() });
        }
        Ok(edit)
    }
}

/// Errors returned by the transaction's reflog-configuration builders.
pub mod with_reflog_previous_oid {
    /// An invalid prepared edit or transaction state for a reflog-configuration builder.
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error("the transaction must be prepared before configuring its reflog")]
        Unprepared,
        #[error("the prepared transaction has no edit named {name:?}")]
        MissingEdit { name: crate::FullName },
        #[error("the prepared transaction has multiple edits named {name:?}")]
        AmbiguousEdit { name: crate::FullName },
        #[error("the prepared edit named {name:?} is not an object update")]
        NotObjectUpdate { name: crate::FullName },
        #[error("the prepared edit named {name:?} is not a symbolic update")]
        NotSymbolicUpdate { name: crate::FullName },
    }
}

/// The error returned by [`Transaction::ensure_ref_lock()`].
pub mod ensure_ref_lock {
    /// The error returned by [`Transaction::ensure_ref_lock()`][super::Transaction::ensure_ref_lock()].
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error("the transaction must be prepared before ensuring a reference lock")]
        Unprepared,
        #[error("the prepared transaction has no edit named {name:?}")]
        MissingEdit { name: crate::FullName },
        #[error("the prepared transaction has multiple edits named {name:?}")]
        AmbiguousEdit { name: crate::FullName },
        #[error("could not acquire the loose-reference lock for {name:?}")]
        LockAcquire {
            source: gix_lock::acquire::Error,
            name: crate::FullName,
        },
    }
}

impl std::fmt::Debug for Transaction<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transaction")
            .field("store", self.store)
            .field("edits", &self.updates.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

///
pub mod prepare;

///
pub mod commit;
