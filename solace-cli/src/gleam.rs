//! Just enough Gleam for the Phase 1a store: split `gleam format`ted source
//! into top-level symbols, parse imports, and find the names a symbol uses.
//! Formatted Gleam puts every top-level item at column 0, which is what makes
//! a line-based splitter reliable. `gleam format` itself is the syntax check.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Fn,
    Type,
    Const,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Fn => "fn",
            Kind::Type => "type",
            Kind::Const => "const",
        }
    }
    pub fn parse(s: &str) -> Kind {
        match s {
            "type" => Kind::Type,
            "const" => Kind::Const,
            _ => Kind::Fn,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    /// Includes the doc comment and any comments directly above it.
    pub text: String,
}

pub struct Module {
    /// Module doc comment and imports: everything before the first item.
    pub header: String,
    pub items: Vec<Item>,
}

/// Kind and name when `line` starts a top-level item.
fn declaration(line: &str) -> Option<(Kind, String)> {
    let mut rest = line.strip_prefix("pub ").unwrap_or(line);
    rest = rest.strip_prefix("opaque ").unwrap_or(rest);
    let (kind, rest) = if let Some(r) = rest.strip_prefix("fn ") {
        (Kind::Fn, r)
    } else if let Some(r) = rest.strip_prefix("type ") {
        (Kind::Type, r)
    } else if let Some(r) = rest.strip_prefix("const ") {
        (Kind::Const, r)
    } else {
        return None;
    };
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!name.is_empty()).then_some((kind, name))
}

/// Comment or attribute lines that belong to the item below them.
fn is_leading(line: &str) -> bool {
    (line.starts_with("//") && !line.starts_with("////")) || line.starts_with('@')
}

pub fn split(source: &str) -> Module {
    let lines: Vec<&str> = source.lines().collect();
    let mut starts = vec![]; // (first line incl. leading comments, kind, name)
    for (i, line) in lines.iter().enumerate() {
        if let Some((kind, name)) = declaration(line) {
            let mut first = i;
            while first > 0 && is_leading(lines[first - 1]) {
                first -= 1;
            }
            starts.push((first, kind, name));
        }
    }
    let header_end = starts.first().map_or(lines.len(), |s| s.0);
    let mut items = vec![];
    for (n, (first, kind, name)) in starts.iter().enumerate() {
        let end = starts.get(n + 1).map_or(lines.len(), |s| s.0);
        let text = lines[*first..end].join("\n").trim_end().to_string();
        items.push(Item { kind: *kind, name: name.clone(), text });
    }
    Module { header: lines[..header_end].join("\n").trim().to_string(), items }
}

/// Constructor names defined by a type's text.
pub fn constructors(type_text: &str) -> Vec<String> {
    type_text
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("  ")?;
            let first = rest.chars().next()?;
            first.is_uppercase().then(|| rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect())
        })
        .collect()
}

/// The parts of a bracketed list, split at its top-level commas: `s` starts
/// just after the opening bracket and ends at the matching close. Each part
/// comes with its byte offset in `s`; blank parts (trailing commas) are left out.
pub fn split_args(s: &str) -> Vec<(usize, String)> {
    let (mut depth, mut parts, mut start) = (0i32, vec![], 0);
    let mut end = s.len();
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => {
                end = i;
                break;
            }
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push((start, s[start..i].to_string()));
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push((start, s[start..end].to_string()));
    parts.into_iter().filter(|(_, p)| !p.trim().is_empty()).collect()
}

/// Each constructor of a type with its fields' labels, in position order.
pub fn record_fields(type_text: &str) -> Vec<(String, Vec<Option<String>>)> {
    let code = strip_comments(type_text);
    let mut out = vec![];
    for (at, line) in code.split_inclusive('\n').scan(0, |at, l| Some((std::mem::replace(at, *at + l.len()), l))) {
        let Some(rest) = line.strip_prefix("  ").filter(|r| r.starts_with(|c: char| c.is_uppercase())) else { continue };
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let open = at + 2 + name.len();
        let fields = if code[open..].starts_with('(') { split_args(&code[open + 1..]) } else { vec![] };
        let label = |p: &str| {
            let (l, _) = p.split_once(':')?;
            let l = l.trim();
            (l.starts_with(|c: char| c.is_lowercase()) && l.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| l.to_string())
        };
        out.push((name, fields.iter().map(|(_, p)| label(p)).collect()));
    }
    out
}

/// The declaration part of an item: a function up to its body, a type or
/// constant in full. Leading comments except the first doc line are dropped.
pub fn signature(item: &Item) -> String {
    let doc = item.text.lines().find(|l| l.starts_with("///")).map(|l| format!("{l}\n"));
    let decl_start = item.text.lines().position(|l| declaration(l).is_some()).unwrap_or(0);
    let decl: String = item.text.lines().skip(decl_start).collect::<Vec<_>>().join("\n");
    let decl = match item.kind {
        Kind::Fn => {
            let mut depth = 0i32;
            let mut cut = decl.len();
            for (i, c) in decl.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    '{' if depth == 0 => {
                        cut = i;
                        break;
                    }
                    _ => {}
                }
            }
            decl[..cut].trim_end().to_string()
        }
        _ => decl,
    };
    doc.unwrap_or_default() + &decl
}

pub struct Import {
    pub module: String,
    pub alias: String,
    /// (name in the imported module, local name)
    pub unqualified: Vec<(String, String)>,
}

pub fn imports(header: &str) -> Vec<Import> {
    let code = strip_comments(header);
    let mut out = vec![];
    let mut rest = code.as_str();
    while let Some(at) = rest.find("import ") {
        let at_line_start = at == 0 || rest[..at].ends_with('\n');
        rest = &rest[at + 7..];
        if !at_line_start {
            continue;
        }
        let module: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '/').collect();
        rest = &rest[module.len()..];
        let mut unqualified = vec![];
        if let Some(r) = rest.strip_prefix(".{") {
            let close = r.find('}').unwrap_or(r.len());
            for part in r[..close].split(',') {
                let part = part.trim();
                let part = part.strip_prefix("type ").unwrap_or(part).trim();
                if part.is_empty() {
                    continue;
                }
                let mut words = part.split(" as ");
                let name = words.next().unwrap().trim().to_string();
                let local = words.next().map_or(name.clone(), |s| s.trim().to_string());
                unqualified.push((name, local));
            }
            rest = &r[(close + 1).min(r.len())..];
        }
        let line_end = rest.find('\n').unwrap_or(rest.len());
        let alias = match rest[..line_end].trim().strip_prefix("as ") {
            Some(a) => a.trim().to_string(),
            None => module.rsplit('/').next().unwrap_or(&module).to_string(),
        };
        out.push(Import { module, alias, unqualified });
    }
    out
}

/// Source with string literals and comments blanked out. Line breaks are
/// kept, so lines still match the source's.
pub fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push('"');
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        chars.next();
                    } else if c == '"' {
                        break;
                    } else if c == '\n' {
                        out.push('\n');
                    }
                }
                out.push('"');
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

pub enum Ref {
    /// `alias.name`
    Qualified(String, String),
    Bare(String),
}

/// Every top-level name a piece of code mentions, as written. Names bound
/// locally (parameters, `let`, `use`, lambda parameters, case patterns) are
/// left out within their scope, so `let tax = tax.quote(x)` still refers to
/// the `tax` module but a later `tax.total` does not. Scopes follow the
/// formatter's layout: a lambda parameter lasts until the lambda's closing
/// brace; a `let`, `use` or case-pattern binding until the indentation drops
/// below the line that introduced it.
pub fn references(code: &str) -> Vec<Ref> {
    scan(code).0.into_iter().map(|r| r.0).collect()
}

/// `references`, each with the line (from 0) it is on.
pub fn references_at(code: &str) -> Vec<(Ref, usize)> {
    scan(code).0
}

/// The record fields a piece of code reads, each with its line: `x.field` on
/// a local `x`, a field of a field or of a call's result (`f(x).field`), and
/// labels in constructor patterns (`Quote(total_cents: t, ..) ->`).
// ponytail: positional pattern fields (`let Square(n) = s`) aren't counted as reads.
pub fn field_reads(code: &str) -> Vec<(String, usize)> {
    scan(code).1
}

fn scan(code: &str) -> (Vec<(Ref, usize)>, Vec<(String, usize)>) {
    let code = strip_comments(code);
    let chars: Vec<char> = code.chars().collect();
    let mut line_of = Vec::with_capacity(chars.len());
    let mut n = 0;
    for &c in &chars {
        line_of.push(n);
        n += (c == '\n') as usize;
    }
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let word_at = |i: usize| -> (String, usize) {
        let mut j = i;
        while j < chars.len() && is_ident(chars[j]) {
            j += 1;
        }
        (chars[i..j].iter().collect(), j)
    };
    let line_end = |i: usize| chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| i + p);
    let bindings = local_bindings(&chars, &line_end);
    let bound = |name: &str, at: usize| bindings.iter().any(|(n, from, to)| n == name && *from <= at && at < *to);
    let lower = |s: &str| s.starts_with(|c: char| c.is_lowercase());

    let (mut refs, mut reads) = (vec![], vec![]);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let starts_ident = (c.is_alphabetic() || c == '_') && (i == 0 || !is_ident(chars[i - 1]));
        if !starts_ident {
            i += 1;
            continue;
        }
        // `..base` in a record update is a value, not a field
        let after_dot = i > 0 && chars[i - 1] == '.' && !(i > 1 && chars[i - 2] == '.');
        let (name, j) = word_at(i);
        let label = j < chars.len() && chars[j] == ':' && chars.get(j + 1) != Some(&':');
        if after_dot && lower(&name) {
            reads.push((name.clone(), line_of[i]));
        }
        if j + 1 < chars.len() && chars[j] == '.' && chars[j + 1].is_alphabetic() && !after_dot {
            let (member, k) = word_at(j + 1);
            if !bound(&name, i) {
                refs.push((Ref::Qualified(name, member), line_of[i]));
            } else if lower(&member) {
                reads.push((member, line_of[i]));
            }
            i = k;
        } else {
            if !after_dot && !label && !bound(&name, i) {
                refs.push((Ref::Bare(name), line_of[i]));
            }
            i = j;
        }
    }
    // labels inside constructor patterns: case arms, and `let`/`use` patterns
    for (n, line) in code.split('\n').enumerate() {
        let t = line.trim_start();
        let pattern = if let Some(rest) = t.strip_prefix("let ").or_else(|| t.strip_prefix("use ")) {
            rest.split(['=', '<']).next().unwrap_or("")
        } else if is_arm(t) {
            &t[..t.find(" ->").unwrap()]
        } else {
            continue;
        };
        let (mut depth, mut word) = (0, String::new());
        for c in pattern.chars() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ':' if depth > 0 && lower(&word) => reads.push((std::mem::take(&mut word), n)),
                _ => {}
            }
            if is_ident(c) { word.push(c) } else if c != ' ' { word.clear() }
        }
    }
    (refs, reads)
}

/// A case arm's line: `Pattern(a, b) ->` (the pattern before the arrow).
fn is_arm(t: &str) -> bool {
    t.find(" ->").is_some_and(|arrow| {
        !t.starts_with("fn") && !t.starts_with("let ") && !t.starts_with("use ") && !t[..arrow].contains('=') && !t[..arrow].contains("fn(")
    })
}

/// (name, first position in scope, first position out of scope), as char
/// positions.
fn local_bindings(chars: &[char], line_end: &dyn Fn(usize) -> usize) -> Vec<(String, usize, usize)> {
    let text: String = chars.iter().collect();
    let len = chars.len();
    let line_start = |i: usize| chars[..i].iter().rposition(|&c| c == '\n').map_or(0, |p| p + 1);
    let indent = |start: usize| chars[start..].iter().take_while(|&&c| c == ' ').count();
    // first later line (non-blank) indented less than `level` (or at most, if `inclusive`)
    let dedent = |from: usize, level: usize, inclusive: bool| -> usize {
        let mut i = line_end(from);
        while i < len {
            let start = i + 1;
            let n = indent(start);
            let blank = start + n >= len || chars[start + n] == '\n';
            if !blank && (n < level || (inclusive && n <= level)) {
                return start;
            }
            i = line_end(start);
        }
        len
    };
    let char_pos = |byte: usize| text[..byte].chars().count();
    let mut out = vec![];
    // let / use
    for (kw, stop) in [("let ", "="), ("use ", "<-")] {
        let mut from = 0;
        while let Some(p) = text[from..].find(kw) {
            let at = from + p;
            from = at + kw.len();
            if at > 0 && text[..at].chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let rest = &text[from..];
            let Some(stop_at) = rest.find(stop) else { continue };
            let pattern = &rest[..stop_at];
            // drop type annotations: `let x: Int =`
            let pattern: String = pattern.split(':').next().unwrap_or("").to_string();
            let at_char = char_pos(at);
            let level = indent(line_start(at_char));
            // bound after the whole right-hand side, which may run over several
            // lines: `use customer <- result.try(\n  customer.Customer(..` still
            // refers to the module
            let bound_from = line_end(char_pos(from + stop_at)).max(dedent(at_char, level, true));
            let bound_to = dedent(at_char, level, false);
            for w in pattern_names(&pattern) {
                // the pattern itself names the binding, not a symbol
                out.push((w.clone(), char_pos(from), char_pos(from + stop_at)));
                out.push((w, bound_from, bound_to));
            }
        }
    }
    // function and lambda parameters
    let mut from = 0;
    while let Some(p) = text[from..].find("fn") {
        let at = from + p;
        from = at + 2;
        let before_ok = at == 0 || !text[..at].chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_');
        let rest = &text[from..];
        let Some(open) = rest.find('(') else { continue };
        let between = rest[..open].trim();
        if !before_ok || !between.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        // the body: from the first `{` after the parameters to its match
        let after_params = {
            let mut d = 0;
            let mut end = rest.len();
            for (k, c) in rest[open..].char_indices() {
                match c {
                    '(' => d += 1,
                    ')' => {
                        d -= 1;
                        if d == 0 {
                            end = open + k + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            end
        };
        let body_end = match rest[after_params..].find('{') {
            Some(b) => {
                let mut d = 0;
                let mut end = text.len();
                for (k, c) in rest[after_params + b..].char_indices() {
                    match c {
                        '{' => d += 1,
                        '}' => {
                            d -= 1;
                            if d == 0 {
                                end = from + after_params + b + k;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                char_pos(end)
            }
            None => len,
        };
        let mut depth = 0;
        let mut param = String::new();
        let mut params = vec![];
        for c in rest[open..].chars() {
            match c {
                '(' => {
                    depth += 1;
                    if depth == 1 {
                        continue;
                    }
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        params.push(std::mem::take(&mut param));
                        break;
                    }
                }
                ',' if depth == 1 => {
                    params.push(std::mem::take(&mut param));
                    continue;
                }
                _ => {}
            }
            param.push(c);
        }
        for p in params {
            let before_type = p.split(':').next().unwrap_or("");
            if let Some(name) = before_type.split_whitespace().last()
                && name.chars().next().is_some_and(|c| c.is_lowercase() || c == '_')
            {
                out.push((name.to_string(), char_pos(at), body_end));
            }
        }
    }
    // case arm patterns: `Pattern(a, b) ->` at the start of a line
    let mut pos = 0;
    for line in text.split('\n') {
        let t = line.trim_start();
        if is_arm(t) {
            let pattern = t[..t.find(" ->").unwrap()].split(" if ").next().unwrap_or("");
            let level = line.len() - t.len();
            let to = dedent(pos, level, true);
            for w in pattern_names(pattern) {
                out.push((w, pos, to));
            }
        }
        pos += line.chars().count() + 1;
    }
    out
}

/// The names a pattern binds: its lowercase words, leaving out the module
/// part of `mod.Ctor`.
pub fn pattern_names(s: &str) -> Vec<String> {
    let mut words = vec![];
    for w in s.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')) {
        for (k, part) in w.split('.').enumerate() {
            if k == 0 && w.contains('.') {
                continue;
            }
            if part.chars().next().is_some_and(|c| c.is_lowercase() || c == '_') && !KEYWORDS.contains(&part) {
                words.push(part.to_string());
            }
        }
    }
    words
}

const KEYWORDS: &[&str] = &["assert", "let", "use", "case", "if", "fn", "pub", "todo", "panic", "as", "echo"];

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "//// Checkout.\n\nimport gleam/list\nimport gleam/order as ordering\nimport pilot/order.{type Order, Line as L}\n\n// section\n\n/// Pays.\npub fn pay(a: Int) -> Result(\n  Int,\n  Nil,\n) {\n  list.map([], fn(x) { L(x) })\n  // helper(\"no\")\n  helper(\"a.b\")\n}\n\n@deprecated(\"x\")\nfn helper(s: String) {\n  s\n}\n\npub type Status {\n  Draft\n  /// doc\n  Cancelled(reason: String)\n}\n\nconst limit = 3\n";

    #[test]
    fn splits_items_with_leading_comments() {
        let m = split(SRC);
        assert!(m.header.starts_with("//// Checkout.") && m.header.contains("import pilot/order"));
        let names: Vec<_> = m.items.iter().map(|i| (i.kind.as_str(), i.name.as_str())).collect();
        assert_eq!(names, [("fn", "pay"), ("fn", "helper"), ("type", "Status"), ("const", "limit")]);
        assert!(m.items[0].text.starts_with("/// Pays."));
        assert!(m.items[1].text.starts_with("@deprecated"));
        assert!(m.header.ends_with("// section"));
    }

    #[test]
    fn signatures_and_constructors() {
        let m = split(SRC);
        assert_eq!(signature(&m.items[0]), "/// Pays.\npub fn pay(a: Int) -> Result(\n  Int,\n  Nil,\n)");
        assert_eq!(constructors(&m.items[2].text), ["Draft", "Cancelled"]);
    }

    #[test]
    fn parses_imports() {
        let m = split(SRC);
        let i = imports(&m.header);
        let got: Vec<_> = i.iter().map(|i| (i.module.as_str(), i.alias.as_str(), i.unqualified.len())).collect();
        assert_eq!(got, [("gleam/list", "list", 0), ("gleam/order", "ordering", 0), ("pilot/order", "order", 2)]);
        assert_eq!(i[2].unqualified[1], ("Line".to_string(), "L".to_string()));
    }

    #[test]
    fn finds_references_outside_strings_and_comments() {
        let m = split(SRC);
        let refs = references(&m.items[0].text);
        let q: Vec<_> = refs.iter().filter_map(|r| if let Ref::Qualified(a, b) = r { Some(format!("{a}.{b}")) } else { None }).collect();
        let bare: Vec<_> = refs.iter().filter_map(|r| if let Ref::Bare(b) = r { Some(b.as_str()) } else { None }).collect();
        assert_eq!(q, ["list.map"]);
        assert!(bare.contains(&"helper") && bare.contains(&"L") && !bare.contains(&"b"));
    }

    #[test]
    fn record_fields_and_field_reads() {
        let ty = "pub type Quote {\n  Quote(\n    /// the lines\n    lines: List(Line),\n    total_cents: Int,\n  )\n  Empty\n  Pair(Int, label: String)\n}";
        let none: Vec<Option<String>> = vec![];
        assert_eq!(record_fields(ty), [
            ("Quote".to_string(), vec![Some("lines".to_string()), Some("total_cents".to_string())]),
            ("Empty".to_string(), none),
            ("Pair".to_string(), vec![None, Some("label".to_string())]),
        ]);
        let code = "pub fn f(q: Quote) {\n  let t = q.total_cents\n  let Quote(lines: ls, ..) = q\n  let r = Quote(..q, total_cents: tax.rate)\n  case g(q).lines {\n    Pair(_, label:) -> label\n  }\n  r.inner.depth\n}";
        let got: Vec<(String, usize)> = field_reads(code);
        let got: Vec<(&str, usize)> = got.iter().map(|(f, l)| (f.as_str(), *l)).collect();
        assert_eq!(got, [("total_cents", 1), ("lines", 4), ("inner", 7), ("depth", 7), ("lines", 2), ("label", 5)]);
    }

    #[test]
    fn locals_shadow_top_level_names() {
        let code = "pub fn f(quote: Quote, shipping: S) -> Int {\n  let tax = tax.quote(1)\n  let assert Ok(#(db, o)) = g(o)\n  list.map([], fn(order) { order.id })\n  case x {\n    Ok(line) if line > 0 -> line + tax.total + db\n    order.Paid -> order.x\n  }\n  shipping.cost + quote.total + h(label: 1)\n}";
        let q: Vec<String> = references(code).iter().filter_map(|r| if let Ref::Qualified(a, b) = r { Some(format!("{a}.{b}")) } else { None }).collect();
        let bare: Vec<String> = references(code).iter().filter_map(|r| if let Ref::Bare(b) = r { Some(b.clone()) } else { None }).collect();
        assert_eq!(q, ["tax.quote", "list.map", "order.Paid", "order.x"]);
        assert!(bare.contains(&"g".to_string()) && bare.contains(&"o".to_string()) && bare.contains(&"h".to_string()));
        assert!(!bare.contains(&"quote".to_string()) && !bare.contains(&"line".to_string()) && !bare.contains(&"label".to_string()));
        // a `use` or `let` pattern binds its names; it doesn't refer to symbols of that name
        let bare: Vec<String> = references("pub fn f() {\n  use request <- try(g())\n  let tax = 1\n  request + tax\n}")
            .iter()
            .filter_map(|r| if let Ref::Bare(b) = r { Some(b.clone()) } else { None })
            .collect();
        assert!(bare.contains(&"g".to_string()) && !bare.contains(&"request".to_string()) && !bare.contains(&"tax".to_string()), "{bare:?}");
        // a binding starts after its whole right-hand side, however many lines it takes
        let code = "pub fn f() {\n  use customer <- result.try(\n    customer.Customer(id: 1)\n    |> customer.validate,\n  )\n  customer.email\n}";
        let q: Vec<String> = references(code).iter().filter_map(|r| if let Ref::Qualified(a, b) = r { Some(format!("{a}.{b}")) } else { None }).collect();
        assert_eq!(q, ["result.try", "customer.Customer", "customer.validate"]);
    }
}
