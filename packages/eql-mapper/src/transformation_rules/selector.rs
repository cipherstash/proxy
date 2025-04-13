use std::marker::PhantomData;

use sqltk::{Context, Visitable};

use crate::EqlMapperError;

/// A `Selector` is triggered during AST traversal when some condition becomes true, and then performs some action in response.
///
/// `Selector` is part of the plumbing for building up modular AST transformation rules.
pub(crate) trait Selector {
    /// The type produced on a successful match.
    type Matched<'ast>;

    /// The type of the node being transformed in a [`sqltk::Transform::transform`] invocation.
    type Target: Visitable;

    /// Determines if `Self` is triggered - taking into account `ctx`, `source_node` & `target_node`, and if so invokes
    /// `then(Self::Matched<'ast>, &'ast mut)` which may or may not mutate `target_node`.
    ///
    /// When the `Selector` is triggered implementations must invoke `then` and return its `Result<(), EqlMapperError>`.
    ///
    /// When the `Selector` is not triggered implementations must return `Ok(())` without invoking `then`.
    ///
    /// Designed to be invoked on every call to a [`sqltk::Transform::transform`] impl.
    fn on_match_then<'ast, N, F>(
        ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
        then: &mut F,
    ) -> Result<N, EqlMapperError>
    where
        N: Visitable,
        F: FnMut(Self::Matched<'ast>, Self::Target) -> Result<N, EqlMapperError>;
}

/// A [`Selector`] impl that matches the trailing N AST node types of a path through an AST up to an including the the
/// type of the target node (the type of the node being transformed in a [`sqltk::Transform`] impl).
///
/// The `Selector` trait is implemented for `MatchTrailing<T>` where `T` is a tuple-type of up to four elements.
///
/// The last element of the tuple is the type of the *target node*, the previous element is its parent node and so on.
/// Therefore you can think of `MatchTrailing` as matching the last N node types of a path through an AST.
pub struct MatchTrailing<T>(PhantomData<T>);

/// A [`Selector`] impl that matches target node of a [`sqltk::Transform::transform`] invocation.
pub struct MatchTarget<T>(PhantomData<T>);

// TODO: write a declarative macro to derive all of these impls.

#[allow(unused)]
impl<C: Visitable> Selector for MatchTrailing<(C,)> {
    type Matched<'a> = (&'a C,);
    type Target = C;

    fn on_match_then<'ast, N, F>(
        ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
        then: &mut F,
    ) -> Result<N, EqlMapperError>
    where
        N: Visitable,
        F: FnMut(Self::Matched<'ast>, Self::Target) -> Result<N, EqlMapperError>,
    {
        let matcher =
            || -> Option<Self::Matched<'ast>> { Some((source_node.downcast_ref::<C>()?,)) };

        if let Some(matched) = matcher() {
            let target_node: Self::Target = unsafe { std::mem::transmute_copy(&target_node)};
            return (then)(matched, target_node);
        }

        Ok(target_node)
    }
}

#[allow(unused)]
impl<P0: Visitable, C: Visitable> Selector for MatchTrailing<(P0, C)> {
    type Matched<'a> = (&'a P0, &'a C);
    type Target = C;

    fn on_match_then<'ast, N, F>(
        ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
        then: &mut F,
    ) -> Result<N, EqlMapperError>
    where
        N: Visitable,
        F: FnMut(Self::Matched<'ast>, Self::Target) -> Result<N, EqlMapperError>,
    {
        let matcher = || -> Option<Self::Matched<'ast>> {
            Some((ctx.nth_last_as(0)?, source_node.downcast_ref::<C>()?))
        };

        if let Some(matched) = matcher() {
            return (then)(matched, target_node);
        }

        Ok(target_node)
    }
}

#[allow(unused)]
impl<P1: Visitable, P0: Visitable, C: Visitable> Selector for MatchTrailing<(P1, P0, C)> {
    type Matched<'a> = (&'a P1, &'a P0, &'a C);
    type Target = C;

    fn on_match_then<'ast, N, F>(
        ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
        then: &mut F,
    ) -> Result<N, EqlMapperError>
    where
        N: Visitable,
        F: FnMut(Self::Matched<'ast>, N) -> Result<N, EqlMapperError>,
    {
        let matcher = || -> Option<Self::Matched<'ast>> {
            Some((
                ctx.nth_last_as(1)?,
                ctx.nth_last_as(0)?,
                source_node.downcast_ref::<C>()?,
            ))
        };

        if let Some(matched) = matcher() {
            return (then)(matched, target_node);
        }

        Ok(target_node)
    }
}

impl<P2: Visitable, P1: Visitable, P0: Visitable, C: Visitable> Selector
    for MatchTrailing<(P2, P1, P0, C)>
{
    type Matched<'a> = (&'a P2, &'a P1, &'a P0, &'a C);
    type Target = C;

    fn on_match_then<'ast, N, F>(
        ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
        then: &mut F,
    ) -> Result<N, EqlMapperError>
    where
        N: Visitable,
        F: FnMut(Self::Matched<'ast>, N) -> Result<N, EqlMapperError>,
    {
        let matcher = || -> Option<Self::Matched<'ast>> {
            Some((
                ctx.nth_last_as(2)?,
                ctx.nth_last_as(1)?,
                ctx.nth_last_as(0)?,
                source_node.downcast_ref::<C>()?,
            ))
        };

        if let Some(matched) = matcher() {
            return (then)(matched, target_node);
        }

        Ok(target_node)
    }
}

impl<C: Visitable> Selector for MatchTarget<C> {
    type Matched<'a> = &'a C;
    type Target = C;

    fn on_match_then<'ast, N, F>(
        _ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
        then: &mut F,
    ) -> Result<N, EqlMapperError>
    where
        N: Visitable,
        F: FnMut(&'ast C, N) -> Result<N, EqlMapperError>,
    {
        let matcher =
            || -> Option<Self::Matched<'ast>> { source_node.downcast_ref::<C>() };

        if let Some(matched) = matcher() {
            return (then)(matched, target_node);
        }

        Ok(target_node)
    }
}