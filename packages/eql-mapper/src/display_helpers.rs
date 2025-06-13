use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    sync::Arc,
};

use sqltk::parser::ast::{
    Delete, Expr, Function, Insert, Query, Select, SelectItem, SetExpr, Statement, Value, Values,
};
use sqltk::NodeKey;

use crate::{unifier::EqlTerm, Param, Type};

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct Fmt<T>(pub(crate) T);

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct FmtAst<T>(pub(crate) T);

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct FmtAstVec<T>(pub(crate) T);

impl Display for Fmt<NodeKey<'_>> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(node) = self.0.get_as::<Statement>() {
            return Display::fmt(&FmtAst(node), f);
        }
        if let Some(node) = self.0.get_as::<Query>() {
            return Display::fmt(&FmtAst(node), f);
        }
        if let Some(node) = self.0.get_as::<Insert>() {
            return Display::fmt(&FmtAst(node), f);
        }
        if let Some(node) = self.0.get_as::<Delete>() {
            return Display::fmt(&FmtAst(node), f);
        }
        if let Some(node) = self.0.get_as::<Expr>() {
            return Display::fmt(&FmtAst(node), f);
        }
        if let Some(node) = self.0.get_as::<SetExpr>() {
            return Display::fmt(&FmtAst(node), f);
        }
        if let Some(node) = self.0.get_as::<Select>() {
            return Display::fmt(&FmtAst(node), f);
        }
        if let Some(node) = self.0.get_as::<SelectItem>() {
            return Display::fmt(&FmtAst(node), f);
        }
        if let Some(node) = self.0.get_as::<Vec<SelectItem>>() {
            return Display::fmt(&FmtAstVec(node), f);
        }
        if let Some(node) = self.0.get_as::<Function>() {
            return Display::fmt(&FmtAst(node), f);
        }
        if let Some(node) = self.0.get_as::<Values>() {
            return Display::fmt(&FmtAst(node), f);
        }
        if let Some(node) = self.0.get_as::<Value>() {
            return Display::fmt(&FmtAst(node), f);
        }

        f.write_str("!! CANNOT RENDER SQL NODE !!!")?;

        Ok(())
    }
}

impl Display for Fmt<&HashMap<NodeKey<'_>, Type>> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out: Vec<String> = Vec::new();
        out.push("{ ".into());
        for (k, v) in self.0.iter() {
            out.push(format!("{}: {}", Fmt(*k), v));
        }
        out.push(" }".into());
        f.write_str(&out.join(", "))
    }
}

impl Display for Fmt<&[Arc<crate::unifier::Type>]> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[")?;
        for (idx, ty) in self.0.iter().enumerate() {
            f.write_fmt(format_args!("{}", ty))?;
            if idx < self.0.len() - 1 {
                f.write_str(", ")?;
            }
        }
        f.write_str("]")
    }
}

impl<T: Display> Display for FmtAstVec<&Vec<T>> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("![")?;
        let children = self
            .0
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        f.write_str(&children)?;
        f.write_str("]!")
    }
}

impl<T: Display> Display for FmtAst<&T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        T::fmt(self.0, f)
    }
}

impl Display for Fmt<&Vec<(Param, crate::Value)>> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let formatted = self
            .0
            .iter()
            .map(|(p, v)| format!("{}: {}", p, v))
            .collect::<Vec<_>>()
            .join(", ");
        f.write_str(&formatted)
    }
}

impl Display for Fmt<&Vec<(EqlTerm, &sqltk::parser::ast::Value)>> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let formatted = self
            .0
            .iter()
            .map(|(e, n)| format!("{}: {}", e, n))
            .collect::<Vec<_>>()
            .join(", ");
        f.write_str(&formatted)
    }
}

impl<T: Display> Display for Fmt<Option<T>> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(t) => t.fmt(f),
            None => Display::fmt("<not initialised>", f),
        }
    }
}
