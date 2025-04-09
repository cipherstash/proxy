use std::marker::PhantomData;

use sqltk::{Context, Visitable};

/// Utility for performing fallible matching on the type of the current node of an AST traversal and an arbitrary number
/// of parent node types.
///
/// This type cannot be instantiated: the methods are associated functions on the implemented on the type.
pub struct EndsWith<T>(PhantomData<T>);

#[allow(unused)]
impl<'ast, C: Visitable> EndsWith<(&C,)> {
    pub(crate) fn try_match<N: Visitable>(
        _: &Context<'ast>,
        current_node: &'ast N,
    ) -> Option<(&'ast C,)> {
        let c = current_node.downcast_ref::<C>()?;

        Some((c,))
    }
}

#[allow(unused)]
impl<'ast, C: Visitable> EndsWith<(&mut C,)> {
    pub(crate) fn try_match<N: Visitable>(
        _: &Context<'ast>,
        current_node: &'ast mut N,
    ) -> Option<(&'ast mut C,)> {
        let c = current_node.downcast_mut::<C>()?;

        Some((c,))
    }
}

#[allow(unused)]
impl<'ast, P0: Visitable, C: Visitable> EndsWith<(&P0, &C)> {
    pub(crate) fn try_match<N: Visitable>(
        context: &Context<'ast>,
        current_node: &'ast N,
    ) -> Option<(&'ast P0, &'ast C)> {
        let p0 = context.nth_last_as::<P0>(0)?;
        let c = current_node.downcast_ref::<C>()?;

        Some((p0, c))
    }
}

#[allow(unused)]
impl<'ast, P0: Visitable, C: Visitable> EndsWith<(&P0, &mut C)> {
    pub(crate) fn try_match<N: Visitable>(
        context: &Context<'ast>,
        current_node: &'ast mut N,
    ) -> Option<(&'ast P0, &'ast mut C)> {
        let p0 = context.nth_last_as::<P0>(0)?;
        let c = current_node.downcast_mut::<C>()?;

        Some((p0, c))
    }
}

#[allow(unused)]
impl<'ast, P1: Visitable, P0: Visitable, C: Visitable> EndsWith<(&P1, &P0, &C)> {
    pub(crate) fn try_match<N: Visitable>(
        context: &Context<'ast>,
        current_node: &'ast N,
    ) -> Option<(&'ast P1, &'ast P0, &'ast C)> {
        let p1 = context.nth_last_as::<P1>(1)?;
        let p0 = context.nth_last_as::<P0>(0)?;
        let c = current_node.downcast_ref::<C>()?;

        Some((p1, p0, c))
    }
}

#[allow(unused)]
impl<'ast, P1: Visitable, P0: Visitable, C: Visitable> EndsWith<(&P1, &P0, &mut C)> {
    pub(crate) fn try_match<N: Visitable>(
        context: &Context<'ast>,
        current_node: &'ast mut N,
    ) -> Option<(&'ast P1, &'ast P0, &'ast mut C)> {
        let p1 = context.nth_last_as::<P1>(1)?;
        let p0 = context.nth_last_as::<P0>(0)?;
        let c = current_node.downcast_mut::<C>()?;

        Some((p1, p0, c))
    }
}

#[allow(unused)]
impl<'ast, P2: Visitable, P1: Visitable, P0: Visitable, C: Visitable>
    EndsWith<(&P2, &P1, &P0, &C)>
{
    pub(crate) fn try_match<N: Visitable>(
        context: &Context<'ast>,
        current_node: &'ast N,
    ) -> Option<(&'ast P2, &'ast P1, &'ast P0, &'ast C)> {
        let p2 = context.nth_last_as::<P2>(2)?;
        let p1 = context.nth_last_as::<P1>(1)?;
        let p0 = context.nth_last_as::<P0>(0)?;
        let c = current_node.downcast_ref::<C>()?;

        Some((p2, p1, p0, c))
    }
}

#[allow(unused)]
impl<'ast, P2: Visitable, P1: Visitable, P0: Visitable, C: Visitable>
    EndsWith<(&P2, &P1, &P0, &mut C)>
{
    pub(crate) fn try_match<N: Visitable>(
        context: &Context<'ast>,
        current_node: &'ast mut N,
    ) -> Option<(&'ast P2, &'ast P1, &'ast P0, &'ast mut C)> {
        let p2 = context.nth_last_as::<P2>(2)?;
        let p1 = context.nth_last_as::<P1>(1)?;
        let p0 = context.nth_last_as::<P0>(0)?;
        let c = current_node.downcast_mut::<C>()?;

        Some((p2, p1, p0, c))
    }
}
