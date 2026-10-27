use std::collections::{BTreeSet, HashSet};
use std::ops::Range;

use super::{can_promote, helper_kind, id_indexes, impl_owner_symbol, is_public};
use crate::Index;



pub(super) fn promote_access(ix: &Index, moved: &[bool], target: &str, newtext: &mut [String], promote: &mut BTreeSet<usize>) {
    let ids = id_indexes(ix);
    let mut owners: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for method in 0..ix.syms.len() {
        if !moved[method] || ix.syms[method].kind_text != "fn" {
            continue;
        }
        let Some(parent) = ix.syms[method].parent.as_deref() else { continue };
        let Some(&at) = ids.get(parent) else { continue };
        if ix.syms[at].kind_text != "impl" {
            continue;
        }
        let Ok(owner) = impl_owner_symbol(ix, at) else { continue };
        if !moved[owner] && !visible_from(&ix.syms[owner].module, target) {
            owners.entry(owner).or_default().push(method);
            if !is_public(&newtext[owner]) {
                promote.insert(owner);
            }
        }
        if ix.used_by[method].iter().any(|&u| !moved[u] && !visible_from(target, &ix.syms[u].module))
            && can_promote(ix, &ids, method)
            && !is_public(&newtext[method])
        {
            promote.insert(method);
        }
        for &used in &ix.uses[method] {
            if !moved[used]
                && can_promote(ix, &ids, used)
                && helper_kind(&ix.syms[used].kind_text)
                && !is_public(&newtext[used])
                && !visible_from(&ix.syms[used].module, target)
            {
                promote.insert(used);
            }
        }
    }
    for (owner, methods) in owners {
        let owner_name = &ix.syms[owner].name;
        let used: HashSet<String> = methods.iter().flat_map(|&m| method_fields(&newtext[m], owner_name)).collect();
        newtext[owner] = promote_fields(&newtext[owner], &used);
    }
    for owner in 0..ix.syms.len() {
        if !moved[owner] || ix.syms[owner].kind_text != "struct" {
            continue;
        }
        newtext[owner] = promote_fields(&newtext[owner], &all_fields(&newtext[owner]));
        promote.insert(owner);
    }
}

fn promote_fields(text: &str, used: &HashSet<String>) -> String {
    let Ok(item) = syn::parse_str::<syn::ItemStruct>(text) else { return text.to_string() };
    let mut edits: Vec<(Range<usize>, String)> = vec![];
    for (index, field) in item.fields.iter().enumerate() {
        let name = field.ident.as_ref().map_or_else(|| index.to_string(), |name| name.to_string());
        if !used.contains(&name) {
            continue;
        }
        match &field.vis {
            syn::Visibility::Public(_) => {}
            syn::Visibility::Inherited => {
                let at = match &field.ident {
                    Some(name) => syn::spanned::Spanned::span(name).byte_range().start,
                    None => syn::spanned::Spanned::span(&field.ty).byte_range().start,
                };
                edits.push((at..at, "pub(crate) ".to_string()));
            }
            syn::Visibility::Restricted(vis) => {
                if vis.path.segments.len() == 1 && vis.path.segments[0].ident == "crate" {
                    continue;
                }
                edits.push((syn::spanned::Spanned::span(&field.vis).byte_range(), "pub(crate) ".to_string()));
            }
        }
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = text.to_string();
    for (range, with) in edits.into_iter().rev() {
        out.replace_range(range, &with);
    }
    out
}

pub(super) fn visible_from(from: &str, to: &str) -> bool {
    from == to || from == "crate" || to.starts_with(&format!("{from}::"))
}

pub(super) fn method_fields(text: &str, owner: &str) -> HashSet<String> {
    let (sig, block) = if let Ok(item) = syn::parse_str::<syn::ItemFn>(text) {
        (item.sig, *item.block)
    } else if let Ok(method) = syn::parse_str::<syn::ImplItemFn>(text) {
        (method.sig, method.block)
    } else {
        return HashSet::new();
    };
    let (receivers, collections) = owner_receivers(&sig, owner);
    let mut visitor = FieldVisitor { owner, receivers, collections, added: vec![], used: HashSet::new() };
    syn::visit::Visit::visit_block(&mut visitor, &block);
    visitor.used
}

struct FieldVisitor<'a> {
    owner: &'a str,
    receivers: HashSet<String>,
    collections: HashSet<String>,
    added: Vec<(String, bool)>,
    used: HashSet<String>,
}

impl<'a> FieldVisitor<'a> {
    fn bind(&mut self, name: String) {
        let existed = self.receivers.contains(&name);
        self.receivers.insert(name.clone());
        self.added.push((name, existed));
    }

    fn restore(&mut self, start: usize) {
        while self.added.len() > start {
            if let Some((name, existed)) = self.added.pop() {
                if existed {
                    self.receivers.insert(name);
                } else {
                    self.receivers.remove(&name);
                }
            }
        }
    }

    fn shadow(&mut self, pattern: &syn::Pat) {
        for name in pattern_bindings(pattern) {
            if self.receivers.remove(&name) {
                self.added.push((name, true));
            }
        }
    }
}

impl<'ast, 'a> syn::visit::Visit<'ast> for FieldVisitor<'a> {
    fn visit_block(&mut self, block: &'ast syn::Block) {
        let start = self.added.len();
        for statement in &block.stmts {
            self.visit_stmt(statement);
        }
        self.restore(start);
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        let start = self.added.len();
        self.shadow(&arm.pat);
        syn::visit::visit_arm(self, arm);
        self.restore(start);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        self.visit_pat(&local.pat);
        let receiver = local.init.as_ref().and_then(|init| {
            self.visit_expr(&init.expr);
            if let Some((_, diverge)) = &init.diverge {
                self.visit_expr(diverge);
            }
            local_owner_binding(&local.pat, &init.expr, self.owner, &self.receivers)
        });
        if let Some(name) = receiver {
            self.bind(name);
        } else {
            self.shadow(&local.pat);
        }
    }

    fn visit_expr_for_loop(&mut self, loop_expr: &'ast syn::ExprForLoop) {
        self.visit_expr(&loop_expr.expr);
        let start = self.added.len();
        if let Some(name) = loop_owner_binding(&loop_expr.pat, &loop_expr.expr, &self.collections) {
            self.bind(name);
        } else {
            self.shadow(&loop_expr.pat);
        }
        self.visit_pat(&loop_expr.pat);
        self.visit_block(&loop_expr.body);
        self.restore(start);
    }

    fn visit_expr_if(&mut self, expr: &'ast syn::ExprIf) {
        let start = self.added.len();
        self.visit_expr(&expr.cond);
        self.visit_block(&expr.then_branch);
        self.restore(start);
        if let Some((_, else_branch)) = &expr.else_branch {
            self.visit_expr(else_branch);
        }
    }

    fn visit_expr_let(&mut self, let_expr: &'ast syn::ExprLet) {
        syn::visit::visit_expr_let(self, let_expr);
        self.shadow(&let_expr.pat);
    }

    fn visit_expr_while(&mut self, expr: &'ast syn::ExprWhile) {
        let start = self.added.len();
        syn::visit::visit_expr_while(self, expr);
        self.restore(start);
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        let start = self.added.len();
        for input in &closure.inputs {
            let owner =
                matches!(input, syn::Pat::Type(typed) if type_is_owner(&typed.ty, self.owner)).then(|| binding_name(input)).flatten();
            if let Some(name) = owner {
                self.bind(name);
            } else {
                self.shadow(input);
            }
        }
        syn::visit::visit_expr_closure(self, closure);
        self.restore(start);
    }

    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if receiver_is_owner(&field.base, self.owner, &self.receivers) {
            self.used.insert(member_name(&field.member));
        }
        syn::visit::visit_expr_field(self, field);
    }

    fn visit_expr_struct(&mut self, literal: &'ast syn::ExprStruct) {
        if path_is_owner(&literal.path, self.owner) {
            for field in &literal.fields {
                self.used.insert(member_name(&field.member));
            }
        }
        syn::visit::visit_expr_struct(self, literal);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if func_path(&call.func).is_some_and(|path| path_is_owner(path, self.owner)) {
            for index in 0..call.args.len() {
                self.used.insert(index.to_string());
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_pat_struct(&mut self, pattern: &'ast syn::PatStruct) {
        if path_is_owner(&pattern.path, self.owner) {
            for field in &pattern.fields {
                self.used.insert(member_name(&field.member));
            }
        }
        syn::visit::visit_pat_struct(self, pattern);
    }

    fn visit_pat_tuple_struct(&mut self, pattern: &'ast syn::PatTupleStruct) {
        if path_is_owner(&pattern.path, self.owner) {
            for index in 0..pattern.elems.len() {
                self.used.insert(index.to_string());
            }
        }
        syn::visit::visit_pat_tuple_struct(self, pattern);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::token::Comma>::parse_terminated;
        if let Ok(exprs) = syn::parse::Parser::parse2(parser, mac.tokens.clone()) {
            for expr in &exprs {
                self.visit_expr(expr);
            }
            return;
        }
        token_fields(&mac.tokens, &self.receivers, &mut self.used);
    }
}

fn type_is_owner_sequence(ty: &syn::Type, owner: &str) -> bool {
    match ty {
        syn::Type::Reference(reference) => type_is_owner_sequence(&reference.elem, owner),
        syn::Type::Paren(paren) => type_is_owner_sequence(&paren.elem, owner),
        syn::Type::Group(group) => type_is_owner_sequence(&group.elem, owner),
        syn::Type::Slice(slice) => type_is_owner(&slice.elem, owner),
        syn::Type::Array(array) => type_is_owner(&array.elem, owner),
        syn::Type::Path(path) => path.path.segments.last().is_some_and(|segment| {
            segment.ident == "Vec"
                && matches!(&segment.arguments, syn::PathArguments::AngleBracketed(args)
                    if args.args.iter().any(|arg| matches!(arg, syn::GenericArgument::Type(t) if type_is_owner(t, owner))))
        }),
        _ => false,
    }
}

fn local_owner_binding(pat: &syn::Pat, expr: &syn::Expr, owner: &str, receivers: &HashSet<String>) -> Option<String> {
    if let syn::Pat::Type(typed) = pat {
        if type_is_owner(&typed.ty, owner) {
            return binding_name(&typed.pat);
        }
        return local_owner_binding(&typed.pat, expr, owner, receivers);
    }
    let name = binding_name(pat)?;
    receiver_is_owner(expr, owner, receivers).then_some(name)
}

fn binding_name(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Ident(ident) if ident.subpat.is_none() => Some(ident.ident.to_string()),
        syn::Pat::Type(typed) => binding_name(&typed.pat),
        _ => None,
    }
}

fn loop_owner_binding(pat: &syn::Pat, expr: &syn::Expr, collections: &HashSet<String>) -> Option<String> {
    let syn::Pat::Ident(ident) = pat else { return None };
    if ident.subpat.is_some() {
        return None;
    }
    let name = for_loop_name(expr)?;
    collections.contains(&name).then(|| ident.ident.to_string())
}

fn for_loop_name(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Reference(reference) => for_loop_name(&reference.expr),
        syn::Expr::Paren(paren) => for_loop_name(&paren.expr),
        syn::Expr::Group(group) => for_loop_name(&group.expr),
        syn::Expr::Path(path) => path.path.get_ident().map(ToString::to_string),
        syn::Expr::MethodCall(call)
            if call.args.is_empty() && (call.method == "iter" || call.method == "iter_mut" || call.method == "into_iter") =>
        {
            for_loop_name(&call.receiver)
        }
        _ => None,
    }
}

fn pattern_bindings(pattern: &syn::Pat) -> Vec<String> {
    let mut bindings = PatternBindings { names: vec![] };
    syn::visit::Visit::visit_pat(&mut bindings, pattern);
    bindings.names
}

struct PatternBindings {
    names: Vec<String>,
}

impl<'ast> syn::visit::Visit<'ast> for PatternBindings {
    fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
        self.names.push(pattern.ident.to_string());
        syn::visit::visit_pat_ident(self, pattern);
    }
}

fn owner_receivers(sig: &syn::Signature, owner: &str) -> (HashSet<String>, HashSet<String>) {
    let mut names = HashSet::new();
    let mut collections = HashSet::new();
    for input in &sig.inputs {
        match input {
            syn::FnArg::Receiver(_) => {
                names.insert("self".to_string());
            }
            syn::FnArg::Typed(arg) => {
                let syn::Pat::Ident(ident) = arg.pat.as_ref() else { continue };
                let name = ident.ident.to_string();
                if type_is_owner(&arg.ty, owner) {
                    names.insert(name);
                } else if type_is_owner_sequence(&arg.ty, owner) {
                    collections.insert(name);
                }
            }
        }
    }
    (names, collections)
}

fn type_is_owner(ty: &syn::Type, owner: &str) -> bool {
    match ty {
        syn::Type::Path(path) => path_is_owner(&path.path, owner),
        syn::Type::Reference(reference) => type_is_owner(&reference.elem, owner),
        syn::Type::Paren(paren) => type_is_owner(&paren.elem, owner),
        syn::Type::Group(group) => type_is_owner(&group.elem, owner),
        syn::Type::Ptr(pointer) => type_is_owner(&pointer.elem, owner),
        _ => false,
    }
}

fn path_is_owner(path: &syn::Path, owner: &str) -> bool {
    path.segments.last().is_some_and(|segment| segment.ident == "Self" || segment.ident == owner)
}

fn member_name(member: &syn::Member) -> String {
    match member {
        syn::Member::Named(name) => name.to_string(),
        syn::Member::Unnamed(index) => index.index.to_string(),
    }
}

fn receiver_is_owner(expr: &syn::Expr, owner: &str, receivers: &HashSet<String>) -> bool {
    match expr {
        syn::Expr::Path(path) => path.path.get_ident().is_some_and(|ident| ident == "self" || receivers.contains(&ident.to_string())),
        syn::Expr::Struct(literal) => path_is_owner(&literal.path, owner),
        syn::Expr::Call(call) => func_path(&call.func).is_some_and(|path| path_is_owner(path, owner)),
        syn::Expr::Paren(paren) => receiver_is_owner(&paren.expr, owner, receivers),
        syn::Expr::Group(group) => receiver_is_owner(&group.expr, owner, receivers),
        syn::Expr::Reference(reference) => receiver_is_owner(&reference.expr, owner, receivers),
        _ => false,
    }
}

fn func_path(expr: &syn::Expr) -> Option<&syn::Path> {
    match expr {
        syn::Expr::Path(path) => Some(&path.path),
        syn::Expr::Paren(paren) => func_path(&paren.expr),
        syn::Expr::Group(group) => func_path(&group.expr),
        _ => None,
    }
}

fn token_fields(tokens: &proc_macro2::TokenStream, receivers: &HashSet<String>, used: &mut HashSet<String>) {
    let trees: Vec<proc_macro2::TokenTree> = tokens.clone().into_iter().collect();
    for (index, tree) in trees.iter().enumerate() {
        if let proc_macro2::TokenTree::Group(group) = tree {
            token_fields(&group.stream(), receivers, used);
        }
        let proc_macro2::TokenTree::Ident(receiver) = tree else { continue };
        let receiver = receiver.to_string();
        if receiver != "self" && !receivers.contains(&receiver) {
            continue;
        }
        if !matches!(trees.get(index + 1), Some(proc_macro2::TokenTree::Punct(punct)) if punct.as_char() == '.') {
            continue;
        }
        match trees.get(index + 2) {
            Some(proc_macro2::TokenTree::Ident(field)) => {
                used.insert(field.to_string());
            }
            Some(proc_macro2::TokenTree::Literal(index)) => {
                if let Ok(index) = index.to_string().parse::<u32>() {
                    used.insert(index.to_string());
                }
            }
            _ => {}
        }
    }
}

fn all_fields(text: &str) -> HashSet<String> {
    let Ok(item) = syn::parse_str::<syn::ItemStruct>(text) else { return HashSet::new() };
    item.fields
        .iter()
        .enumerate()
        .map(|(index, field)| field.ident.as_ref().map_or_else(|| index.to_string(), |name| name.to_string()))
        .collect()
}