// Copyright Materialize, Inc. and contributors. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Utilities for transformation of [crate::plan::Plan] structures.

use std::collections::BTreeSet;

use mz_ore::stack::RecursionLimitError;
use mz_repr::GlobalId;

use crate::plan::Plan;
use crate::plan::interpret::{BoundedLattice, FoldMut, Interpreter};

/// The type of configuration options passed to all [Transform::transform] calls
/// as an immutable reference.
#[derive(Debug)]
pub struct TransformConfig {
    /// TODO(database-issues#7533): Add documentation.
    pub monotonic_ids: BTreeSet<GlobalId>,
}

/// A transform for [crate::plan::Plan] nodes.
pub trait Transform<T = mz_repr::Timestamp> {
    /// TODO(database-issues#7533): Add documentation.
    fn name(&self) -> &'static str;

    /// Transform a [Plan] using the given [TransformConfig].
    ///
    /// The default implementation of method just handles plan tracing and
    /// delegates to the [Transform::do_transform] method. Clients should
    /// override this method if they don't want the [Transform::transform] call
    /// to record a trace of its output.
    fn transform(
        &self,
        config: &TransformConfig,
        plan: &mut Plan<T>,
    ) -> Result<(), RecursionLimitError> {
        let _span = tracing::span!(target: "optimizer",
            tracing::Level::TRACE,
            "transform",
            path.segment = self.name(),
        )
        .entered();
        self.do_transform(config, plan)
    }

    /// A method that performs the actual transform.
    fn do_transform(
        &self,
        config: &TransformConfig,
        plan: &mut Plan<T>,
    ) -> Result<(), RecursionLimitError>;
}

/// TODO(database-issues#7533): Add documentation.
pub trait BottomUpTransform<T = mz_repr::Timestamp> {
    /// A type representing analysis information to be associated with each
    /// sub-term and exposed to the transformation action callback.
    type Info: BoundedLattice + Clone;

    /// A type responsible for synthesizing the [Self::Info] associated with
    /// each sub-term.
    type Interpreter<'a>: Interpreter<T, Domain = Self::Info>;

    /// The name for this transform.
    fn name(&self) -> &'static str;

    /// Derive a [Self::Interpreter] instance from the [TransformConfig].
    fn interpreter(config: &TransformConfig) -> Self::Interpreter<'_>;

    /// A callback for manipulating the root of the given [Plan] using the
    /// [Self::Info] derived for itself and its children.
    fn action(plan: &mut Plan<T>, plan_info: &Self::Info, input_infos: &[Self::Info]);
}

impl<A, T> Transform<T> for A
where
    A: BottomUpTransform<T>,
{
    fn name(&self) -> &'static str {
        self.name()
    }

    fn do_transform(
        &self,
        config: &TransformConfig,
        plan: &mut Plan<T>,
    ) -> Result<(), RecursionLimitError> {
        let mut fold = FoldMut::new(Self::interpreter(config), Self::action);
        fold.apply(plan).map(|_| ())
    }
}
