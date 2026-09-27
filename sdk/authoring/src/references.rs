//! Resolve local names to existing live slots without guessing types or values.
use crate::{Literal, Reference, source::SourceIndex};
use std::collections::BTreeMap;
use syn::{
    spanned::Spanned,
    visit::{self, Visit},
};

#[derive(Clone)]
enum Origin {
    Literal(usize),
    Fields(BTreeMap<String, Origin>),
}

pub(crate) fn resolve(source: &str, file: &syn::File, literals: &[Literal]) -> Vec<Reference> {
    if literals.is_empty() {
        return Vec::new();
    }
    let mut resolver = Resolver {
        source,
        index: SourceIndex::new(source),
        literals: literals
            .iter()
            .map(|l| ((l.range.start, l.range.end), l.id))
            .collect(),
        scopes: Vec::new(),
        references: Vec::new(),
    };
    resolver.visit_file(file);
    resolver.references.sort_by_key(|r| r.range.start);
    resolver
        .references
        .dedup_by_key(|r| (r.range.start, r.range.end));
    resolver.references
}

struct Resolver<'a> {
    source: &'a str,
    index: SourceIndex,
    literals: BTreeMap<(usize, usize), usize>,
    scopes: Vec<BTreeMap<String, Option<Origin>>>,
    references: Vec<Reference>,
}
impl Resolver<'_> {
    fn lookup(&self, name: &str) -> Option<Origin> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(name))
            .cloned()
            .flatten()
    }
    fn origin(&self, expr: &syn::Expr) -> Option<Origin> {
        let range = self.index.range(expr.span());
        if let Some(id) = self.literals.get(&(range.start, range.end)) {
            return Some(Origin::Literal(*id));
        }
        match expr {
            syn::Expr::Path(p) if p.qself.is_none() => {
                p.path.get_ident().and_then(|n| self.lookup(&n.to_string()))
            }
            syn::Expr::Paren(p) => self.origin(&p.expr),
            syn::Expr::Group(p) => self.origin(&p.expr),
            syn::Expr::Field(f) => match self.origin(&f.base)? {
                Origin::Fields(fields) => fields.get(&member(&f.member)).cloned(),
                _ => None,
            },
            syn::Expr::Struct(s) => Some(Origin::Fields(
                s.fields
                    .iter()
                    .filter_map(|f| self.origin(&f.expr).map(|o| (member(&f.member), o)))
                    .collect(),
            )),
            syn::Expr::Tuple(t) => Some(Origin::Fields(
                t.elems
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| self.origin(e).map(|o| (i.to_string(), o)))
                    .collect(),
            )),
            _ => None,
        }
    }
    fn record(&mut self, span: proc_macro2::Span, id: usize) {
        if self.references.len() >= 8192 {
            return;
        }
        let range = self.index.range(span);
        self.references.push(Reference {
            literal: id,
            name: self.source[range.start..range.end].into(),
            range,
        });
    }
    fn bind(&mut self, pat: &syn::Pat, origin: Option<Origin>) {
        match pat {
            syn::Pat::Ident(p) => {
                let origin = if p.mutability.is_none() && p.by_ref.is_none() && p.subpat.is_none() {
                    origin
                } else {
                    None
                };
                if let Some(Origin::Literal(id)) = &origin {
                    self.record(p.ident.span(), *id);
                }
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(p.ident.to_string(), origin);
                }
                if let Some((_, subpat)) = &p.subpat {
                    self.bind(subpat, None);
                }
            }
            syn::Pat::Type(p) => self.bind(&p.pat, origin),
            syn::Pat::Paren(p) => self.bind(&p.pat, origin),
            syn::Pat::Tuple(p) => {
                let fields = match origin {
                    Some(Origin::Fields(f))
                        if !p.elems.iter().any(|p| matches!(p, syn::Pat::Rest(_))) =>
                    {
                        f
                    }
                    _ => BTreeMap::new(),
                };
                for (i, pat) in p.elems.iter().enumerate() {
                    self.bind(pat, fields.get(&i.to_string()).cloned());
                }
            }
            syn::Pat::Struct(p) => {
                let fields = match origin {
                    Some(Origin::Fields(f)) => f,
                    _ => BTreeMap::new(),
                };
                for f in &p.fields {
                    self.bind(&f.pat, fields.get(&member(&f.member)).cloned());
                }
            }
            // Unknown destructuring still shadows outer bindings.
            _ => {
                struct Names(Vec<syn::Ident>);
                impl<'ast> Visit<'ast> for Names {
                    fn visit_pat_ident(&mut self, p: &'ast syn::PatIdent) {
                        self.0.push(p.ident.clone());
                        visit::visit_pat_ident(self, p);
                    }
                }
                let mut names = Names(Vec::new());
                names.visit_pat(pat);
                if let Some(scope) = self.scopes.last_mut() {
                    for name in names.0 {
                        scope.insert(name.to_string(), None);
                    }
                }
            }
        }
    }
    fn push(&mut self) {
        self.scopes.push(BTreeMap::new());
    }
    fn pop(&mut self) {
        self.scopes.pop();
    }
}
fn member(member: &syn::Member) -> String {
    match member {
        syn::Member::Named(n) => n.to_string(),
        syn::Member::Unnamed(i) => i.index.to_string(),
    }
}
impl<'ast> Visit<'ast> for Resolver<'_> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        // Nested items cannot capture the enclosing function's locals.
        let outer = std::mem::take(&mut self.scopes);
        if f.sig.constness.is_none() {
            self.visit_block(&f.block);
        }
        self.scopes = outer;
    }
    fn visit_item_const(&mut self, _: &'ast syn::ItemConst) {}
    fn visit_item_static(&mut self, _: &'ast syn::ItemStatic) {}
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        let outer = std::mem::take(&mut self.scopes);
        if f.sig.constness.is_none() {
            self.visit_block(&f.block);
        }
        self.scopes = outer;
    }
    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        let outer = std::mem::take(&mut self.scopes);
        if let Some(block) = &f.default {
            self.visit_block(block);
        }
        self.scopes = outer;
    }
    fn visit_type(&mut self, _: &'ast syn::Type) {}
    fn visit_pat(&mut self, _: &'ast syn::Pat) {}
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}
    fn visit_generic_argument(&mut self, _: &'ast syn::GenericArgument) {}
    fn visit_expr_const(&mut self, _: &'ast syn::ExprConst) {}
    fn visit_expr_reference(&mut self, _: &'ast syn::ExprReference) {}
    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.push();
        // Block items are in scope before their declaration, unlike let bindings.
        for stmt in &block.stmts {
            if let syn::Stmt::Item(item) = stmt {
                let ident = match item {
                    syn::Item::Const(i) => Some(&i.ident),
                    syn::Item::Static(i) => Some(&i.ident),
                    syn::Item::Fn(i) => Some(&i.sig.ident),
                    syn::Item::Struct(i) => Some(&i.ident),
                    _ => None,
                };
                if let Some(i) = ident {
                    self.scopes
                        .last_mut()
                        .expect("block")
                        .insert(i.to_string(), None);
                }
                // Imports may introduce arbitrary value names. Avoid guessing
                // which captured binding they shadow in this block.
                if matches!(item, syn::Item::Use(_)) {
                    let names: Vec<_> =
                        self.scopes.iter().flat_map(|s| s.keys().cloned()).collect();
                    for name in names {
                        self.scopes.last_mut().expect("block").insert(name, None);
                    }
                }
            }
        }
        visit::visit_block(self, block);
        self.pop();
    }
    fn visit_local(&mut self, local: &'ast syn::Local) {
        let mut origin = None;
        if let Some(init) = &local.init {
            self.visit_expr(&init.expr);
            origin = self.origin(&init.expr);
            if let Some((_, otherwise)) = &init.diverge {
                self.visit_expr(otherwise);
                origin = None;
            }
        }
        self.bind(&local.pat, origin);
    }
    fn visit_expr(&mut self, expr: &'ast syn::Expr) {
        if matches!(expr, syn::Expr::Path(_) | syn::Expr::Field(_))
            && let Some(Origin::Literal(id)) = self.origin(expr)
        {
            self.record(expr.span(), id);
            return;
        }
        visit::visit_expr(self, expr);
    }
    fn visit_expr_closure(&mut self, c: &'ast syn::ExprClosure) {
        self.push();
        for p in &c.inputs {
            self.bind(p, None);
        }
        self.visit_expr(&c.body);
        self.pop();
    }
    fn visit_expr_for_loop(&mut self, f: &'ast syn::ExprForLoop) {
        self.visit_expr(&f.expr);
        self.push();
        self.bind(&f.pat, None);
        self.visit_block(&f.body);
        self.pop();
    }
    fn visit_expr_if(&mut self, f: &'ast syn::ExprIf) {
        self.push();
        self.visit_expr(&f.cond);
        self.visit_block(&f.then_branch);
        self.pop();
        if let Some((_, e)) = &f.else_branch {
            self.visit_expr(e);
        }
    }
    fn visit_expr_while(&mut self, f: &'ast syn::ExprWhile) {
        self.push();
        self.visit_expr(&f.cond);
        self.visit_block(&f.body);
        self.pop();
    }
    fn visit_expr_let(&mut self, e: &'ast syn::ExprLet) {
        self.visit_expr(&e.expr);
        self.bind(&e.pat, None);
    }
    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        self.push();
        self.bind(&arm.pat, None);
        if let Some((_, guard)) = &arm.guard {
            self.visit_expr(guard);
        }
        self.visit_expr(&arm.body);
        self.pop();
    }
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*call.func
            && let Some(s) = p.path.segments.last()
        {
            let name = s.ident.to_string();
            if name.starts_with("remember") {
                return;
            }
            if name == "key" {
                if let Some(content) = call.args.last() {
                    self.visit_expr(content);
                }
                return;
            }
        }
        // The callee is not a value argument, even if its name is shadowed.
        for arg in &call.args {
            self.visit_expr(arg);
        }
    }
    fn visit_expr_repeat(&mut self, repeat: &'ast syn::ExprRepeat) {
        self.visit_expr(&repeat.expr);
    }
}
