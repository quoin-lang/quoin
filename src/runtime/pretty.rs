//! Structural value pretty-printing (`.pp`): a width-aware, canonical dump of a value graph
//! for debugging/inspection — distinct from `.s` (human/UI string) and from the Rust `Display`
//! impl (Rust-level debugging). `.pp` never calls `.s`: it renders value types and collections
//! intrinsically, escapes + quotes strings, and shows an object's instance variables. It is a
//! single entrypoint that encodes the whole graph itself — no per-element method dispatch — so
//! the output is canonical (and not overridable from Quoin).
//!
//! Layout uses a Wadler/Leijen document algebra: each collection is a `Group` that prints flat
//! when it fits the target width, else breaks onto indented lines.

use crate::ansi_colorizer;
use crate::introspect::{self, MethodVariant};
use crate::runtime::big_decimal::NativeBigDecimal;
use crate::runtime::big_integer::NativeBigInteger;
use crate::runtime::date_time::NativeDateTime;
use crate::runtime::duration::NativeDuration;
use crate::runtime::ids::{NativeUlid, NativeUuid};
use crate::runtime::list::NativeListState;
use crate::runtime::map::{NativeKeyValuePairState, NativeMapState};
use crate::runtime::method::NativeMethodState;
use crate::runtime::regex::NativeRegexState;
use crate::runtime::set::NativeSetState;
use crate::runtime::time_zone::NativeTimeZone;
use crate::runtime::timestamp::NativeTimestamp;
use crate::value::{Object, ObjectPayload, Value};

use gc_arena::{Gc, lock::RefLock};
use quoin_syntax::highlight::{HighlightType, colors_for};
use std::collections::HashSet;

/// The structural shape a native class contributes to a `.pp` dump: delimiters plus its child
/// values (already lifetime-correct), which the walker lays out and recurses into. Implementing
/// this trait is how a native collection type joins `.pp` (see `NativeListState` etc.).
pub enum PpShape<'gc> {
    /// A sequence — `open item … close` (List `#( )`, Set `#< >`).
    Seq {
        open: &'static str,
        close: &'static str,
        items: Vec<Value<'gc>>,
    },
    /// Keyed entries rendered `key: value` (Map `#{ }`). The bool marks a
    /// STRING key the walker quotes; non-string keys arrive pre-rendered
    /// (structural, unquoted).
    Entries {
        open: &'static str,
        close: &'static str,
        entries: Vec<(String, bool, Value<'gc>)>,
    },
    /// A named record `Name{ field: value … }` with *unquoted* struct-field labels (distinct
    /// from `Entries`, whose keys are quoted as map literals). This is the structural form for
    /// scalar natives (DateTime/UUID/Duration/…) and for `KeyValuePair` — a synthetic decomposition
    /// of the value's own state, never its `.s` string.
    Record {
        name: &'static str,
        fields: Vec<(String, PpChild<'gc>)>,
    },
}

/// One field of a `Record`: either a live Quoin value the walker recurses into, or a text token
/// the native type pre-rendered itself. The latter exists because `pp_shape` has no `Mutation`,
/// so it cannot allocate a Quoin `String` for a synthetic field (a zone name, a UUID's hex, a
/// `BigInt`'s magnitude) — it formats the text directly and the walker emits it as a styled leaf.
pub enum PpChild<'gc> {
    Val(Value<'gc>),
    Text(String, PpRole),
}

/// The token role for a `PpChild::Text` leaf, so a native type can color a field it formatted
/// itself without reaching into the (private) full render palette.
#[derive(Clone, Copy)]
pub enum PpRole {
    /// A string-ish field — quoted like a string literal (`'America/New_York'`).
    Str,
    /// A numeric field too wide for an `Int` (a `BigInt` magnitude, a `Decimal` mantissa).
    Number,
}

pub trait PrettyPrint {
    fn pp_shape<'gc>(&self) -> PpShape<'gc>;
}

// ---- Wadler/Leijen layout algebra ----

enum Doc {
    Text(String),
    /// Flat: a single space; broken: newline + current indent.
    Line,
    /// Flat: nothing; broken: newline + current indent.
    Soft,
    Cat(Vec<Doc>),
    Nest(usize, Box<Doc>),
    Group(Box<Doc>),
    /// A `Role`-colored span. Transparent to layout (the ANSI escapes are zero-width); the
    /// codes are emitted only when `best` is asked to colorize.
    Styled(Role, Box<Doc>),
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Flat,
    Break,
}

/// A pretty-print token role, mapped to the same `HighlightType` palette as input highlighting
/// so a value's `.pp` is colored like the source you'd type.
#[derive(Clone, Copy)]
enum Role {
    Delim,     // collection sigils + structural braces/parens  -> CollectionBrace
    ClassName, // class names                                   -> Global
    Reserved,  // `true` / `false` / `nil`                      -> Global (as in the highlighter)
    Ivar,      // `@field` labels                               -> InstanceIdentifier
    Selector,  // method signatures inside `Method(…)`          -> MethodSignature
    Number,    // Int / Double                                  -> NumberLiteral
    Str,       // strings                                       -> StringLiteral
    Symbol,    // `#sym`                                        -> SymbolLiteral
    Regex,     // `#/…/`                                       -> RegexLiteral
}

impl Role {
    fn highlight(self) -> HighlightType {
        match self {
            Role::Delim => HighlightType::CollectionBrace,
            Role::ClassName | Role::Reserved => HighlightType::Global,
            Role::Ivar => HighlightType::InstanceIdentifier,
            Role::Selector => HighlightType::MethodSignature,
            Role::Number => HighlightType::NumberLiteral,
            Role::Str => HighlightType::StringLiteral,
            Role::Symbol => HighlightType::SymbolLiteral,
            Role::Regex => HighlightType::RegexLiteral,
        }
    }

    /// The ANSI SGR start sequence for this role (first color of its palette entry).
    fn sgr(self) -> String {
        ansi_colorizer::sgr(colors_for(self.highlight())[0])
    }
}

fn text(s: impl Into<String>) -> Doc {
    Doc::Text(s.into())
}

fn styled(role: Role, doc: Doc) -> Doc {
    Doc::Styled(role, Box::new(doc))
}

/// Wrap `items` between `open`/`close` (with a `prefix` like a class name) as a breakable
/// group: flat `pre#(a b c)`; broken, one item per indented line with `pre#(` / `)` on their
/// own lines.
fn bracket(prefix: &str, open: &str, close: &str, items: Vec<Doc>) -> Doc {
    // `prefix` (a class name) and the delimiters are colored; the items color themselves.
    let opener = if prefix.is_empty() {
        styled(Role::Delim, text(open))
    } else {
        Doc::Cat(vec![
            styled(Role::ClassName, text(prefix)),
            styled(Role::Delim, text(open)),
        ])
    };
    let closer = styled(Role::Delim, text(close));
    if items.is_empty() {
        return Doc::Cat(vec![opener, closer]);
    }
    let mut inner = Vec::with_capacity(items.len() * 2);
    for (i, it) in items.into_iter().enumerate() {
        if i > 0 {
            inner.push(Doc::Line);
        }
        inner.push(it);
    }
    Doc::Group(Box::new(Doc::Cat(vec![
        opener,
        Doc::Nest(2, Box::new(Doc::Cat(vec![Doc::Soft, Doc::Cat(inner)]))),
        Doc::Soft,
        closer,
    ])))
}

/// Does the doc on `stack` fit in `remaining` columns laid out flat (until a forced break)?
fn fits(mut remaining: isize, mut stack: Vec<(usize, Mode, &Doc)>) -> bool {
    while remaining >= 0 {
        let (indent, mode, doc) = match stack.pop() {
            Some(x) => x,
            None => return true,
        };
        match doc {
            Doc::Text(s) => remaining -= s.chars().count() as isize,
            Doc::Line => match mode {
                Mode::Flat => remaining -= 1,
                Mode::Break => return true,
            },
            Doc::Soft => {
                if mode == Mode::Break {
                    return true;
                }
            }
            Doc::Cat(ds) => {
                for d in ds.iter().rev() {
                    stack.push((indent, mode, d));
                }
            }
            Doc::Nest(n, d) => stack.push((indent + n, mode, d.as_ref())),
            Doc::Group(d) => stack.push((indent, Mode::Flat, d.as_ref())),
            // A style adds no width — measure its content only.
            Doc::Styled(_, d) => stack.push((indent, mode, d.as_ref())),
        }
    }
    false
}

/// A layout work item: a doc to lay out, or a raw string to emit verbatim (an ANSI escape —
/// written without advancing the column, since it has no visible width).
enum Step<'a> {
    Doc(usize, Mode, &'a Doc),
    Emit(String),
}

/// Lay `doc` out to a string, breaking each group that doesn't fit in `width` columns. With
/// `colorize`, `Styled` spans are wrapped in their role's ANSI escapes (zero-width, so the
/// layout is identical to the uncolored output).
fn best(width: usize, doc: &Doc, colorize: bool) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    let mut stack: Vec<Step> = vec![Step::Doc(0, Mode::Break, doc)];
    while let Some(step) = stack.pop() {
        let (indent, mode, doc) = match step {
            Step::Emit(s) => {
                out.push_str(&s); // zero-width: do not touch `col`
                continue;
            }
            Step::Doc(i, m, d) => (i, m, d),
        };
        match doc {
            Doc::Text(s) => {
                out.push_str(s);
                col += s.chars().count();
            }
            Doc::Line if mode == Mode::Flat => {
                out.push(' ');
                col += 1;
            }
            Doc::Line | Doc::Soft => match mode {
                // `Soft` flat = nothing; both break the same way.
                Mode::Flat => {}
                Mode::Break => {
                    out.push('\n');
                    out.extend(std::iter::repeat_n(' ', indent));
                    col = indent;
                }
            },
            Doc::Cat(ds) => {
                for d in ds.iter().rev() {
                    stack.push(Step::Doc(indent, mode, d));
                }
            }
            Doc::Nest(n, d) => stack.push(Step::Doc(indent + n, mode, d.as_ref())),
            Doc::Group(d) => {
                let fit = fits(
                    width as isize - col as isize,
                    vec![(indent, Mode::Flat, d.as_ref())],
                );
                let m = if fit { Mode::Flat } else { Mode::Break };
                stack.push(Step::Doc(indent, m, d.as_ref()));
            }
            Doc::Styled(role, d) => {
                if colorize {
                    // start … inner … reset, in LIFO push order.
                    stack.push(Step::Emit(ansi_colorizer::SGR_RESET.to_string()));
                    stack.push(Step::Doc(indent, mode, d.as_ref()));
                    stack.push(Step::Emit(role.sgr()));
                } else {
                    stack.push(Step::Doc(indent, mode, d.as_ref()));
                }
            }
        }
    }
    out
}

// ---- value -> Doc walk ----

/// Pretty-print `value` to a string laid out within `width` columns. The single canonical
/// entrypoint: walks the whole graph here (no method dispatch), cycle-guarded. With `colorize`,
/// tokens are ANSI-colored with the input-highlighting palette (for terminal display).
pub fn render<'gc>(value: Value<'gc>, width: usize, colorize: bool) -> String {
    let mut visited = HashSet::new();
    best(width, &value_to_doc(value, &mut visited), colorize)
}

fn value_to_doc<'gc>(value: Value<'gc>, visited: &mut HashSet<usize>) -> Doc {
    match value {
        Value::Int(i) => styled(Role::Number, text(i.to_string())),
        Value::Double(d) => styled(Role::Number, text(format!("{d}"))),
        Value::Bool(b) => styled(Role::Reserved, text(if b { "true" } else { "false" })),
        Value::Nil => styled(Role::Reserved, text("nil")),
        Value::Class(c) => Doc::Cat(vec![
            text("class "),
            styled(Role::ClassName, text(c.borrow().name.to_string())),
        ]),
        Value::ClassMeta(c) => Doc::Cat(vec![
            text("class "),
            styled(Role::ClassName, text(c.borrow().name.to_string())),
            text(" meta"),
        ]),
        Value::Object(o) => {
            let id = Gc::as_ptr(o) as usize;
            let cname = value.class_name();
            // Cycle guard: elide a node already on the current path.
            if !visited.insert(id) {
                return text(format!("{cname}{{…}}"));
            }
            let doc = object_doc(value, o, &cname, visited);
            visited.remove(&id);
            doc
        }
    }
}

/// Render an `Object` value: strings/symbols/bytes/blocks intrinsically, native collections
/// via their `PrettyPrint` shape, and a plain instance as `ClassName{ @ivar: value … }`.
fn object_doc<'gc>(
    value: Value<'gc>,
    o: Gc<'gc, RefLock<Object<'gc>>>,
    cname: &str,
    visited: &mut HashSet<usize>,
) -> Doc {
    // Classify the payload (extracting any children) without holding the borrow across the
    // recursion below.
    enum Payload<'gc> {
        Str(String),
        Sym(String),
        Bytes(Vec<u8>, usize),
        Block(Option<String>),
        Ivars(Vec<(String, Value<'gc>)>),
        Native,
    }
    let payload = {
        let b = o.borrow();
        match &b.payload {
            ObjectPayload::String(s) => Payload::Str((**s).clone()),
            ObjectPayload::Symbol(s) => Payload::Sym((**s).clone()),
            ObjectPayload::Bytes(by) => {
                Payload::Bytes(by.iter().take(16).copied().collect(), by.len())
            }
            ObjectPayload::Block(blk) => Payload::Block(blk.template.name.clone()),
            ObjectPayload::Instance => {
                // Instance vars in slot (declaration) order, mirroring `introspect::describe_value`.
                let cls = b.class.borrow();
                let mut slots: Vec<(String, usize)> = cls
                    .field_slots
                    .iter()
                    .map(|(n, &s)| (n.clone(), s))
                    .collect();
                slots.sort_by_key(|x| x.1);
                let ivars = slots
                    .into_iter()
                    .filter_map(|(n, s)| b.fields.get(s).map(|v| (n, *v)))
                    .collect();
                Payload::Ivars(ivars)
            }
            ObjectPayload::NativeState(_) => Payload::Native,
        }
    };
    match payload {
        Payload::Str(s) => styled(Role::Str, text(quote(&s))),
        Payload::Sym(s) => styled(Role::Symbol, text(format!("#{s}"))),
        Payload::Bytes(preview, len) => text(bytes_repr(&preview, len)),
        Payload::Block(name) => text(match name {
            Some(n) => format!("<block {n}>"),
            None => "<block>".to_string(),
        }),
        Payload::Ivars(ivars) => {
            let items = ivars
                .into_iter()
                .map(|(n, v)| {
                    Doc::Cat(vec![
                        styled(Role::Ivar, text(format!("@{n}"))),
                        text(": "),
                        value_to_doc(v, visited),
                    ])
                })
                .collect();
            bracket(cname, "{", "}", items)
        }
        Payload::Native => native_doc(value, cname, visited),
    }
}

/// The `PpShape` for a native object, dispatched by class name (collections + scalar value-likes),
/// or `None` for a non-collection native (`Regex`/`Method`/opaque). Shared by the renderer
/// (`native_doc`) and the debugger's expandable-variables walker (`value_children`).
fn value_native_shape<'gc>(value: Value<'gc>, cname: &str) -> Option<PpShape<'gc>> {
    match cname {
        "List" => value
            .with_native_state::<NativeListState, _, _>(|s| s.pp_shape())
            .ok(),
        "Set" => value
            .with_native_state::<NativeSetState, _, _>(|s| s.pp_shape())
            .ok(),
        "Map" => value
            .with_native_state::<NativeMapState, _, _>(|s| s.pp_shape())
            .ok(),
        "KeyValuePair" => value
            .with_native_state::<NativeKeyValuePairState, _, _>(|s| s.pp_shape())
            .ok(),
        "Match" => value
            .with_native_state::<crate::runtime::regex::NativeMatchState, _, _>(|s| s.pp_shape())
            .ok(),
        "Date" => value
            .with_native_state::<crate::runtime::civil::NativeDate, _, _>(|s| s.pp_shape())
            .ok(),
        "Time" => value
            .with_native_state::<crate::runtime::civil::NativeTime, _, _>(|s| s.pp_shape())
            .ok(),
        "Span" => value
            .with_native_state::<crate::runtime::span::NativeSpan, _, _>(|s| s.pp_shape())
            .ok(),
        "[OS]Process" => value
            .with_native_state::<crate::runtime::process::NativeProcess, _, _>(|s| s.pp_shape())
            .ok(),
        // Scalar value-like natives: each decomposes its own state into a structural `Record`.
        "DateTime" => value
            .with_native_state::<NativeDateTime, _, _>(|s| s.pp_shape())
            .ok(),
        "Timestamp" => value
            .with_native_state::<NativeTimestamp, _, _>(|s| s.pp_shape())
            .ok(),
        "Duration" => value
            .with_native_state::<NativeDuration, _, _>(|s| s.pp_shape())
            .ok(),
        "TimeZone" => value
            .with_native_state::<NativeTimeZone, _, _>(|s| s.pp_shape())
            .ok(),
        "BigDecimal" => value
            .with_native_state::<NativeBigDecimal, _, _>(|s| s.pp_shape())
            .ok(),
        "BigInteger" => value
            .with_native_state::<NativeBigInteger, _, _>(|s| s.pp_shape())
            .ok(),
        "UUID" => value
            .with_native_state::<NativeUuid, _, _>(|s| s.pp_shape())
            .ok(),
        "ULID" => value
            .with_native_state::<NativeUlid, _, _>(|s| s.pp_shape())
            .ok(),
        _ => None,
    }
}

/// The expandable children of a value for the DAP `variables` view — `(label, child)` in display
/// order, mirroring the inline renderer so the IDE-expanded tree matches what's shown inline. An
/// instance yields its `@ivars` (declaration order); a native collection yields its `pp_shape`
/// children (`[i]` for sequences, keys for maps, fields for records); scalars, strings, blocks,
/// and opaque natives yield none (they render as a single leaf).
pub fn value_children<'gc>(value: Value<'gc>) -> Vec<(String, PpChild<'gc>)> {
    let Value::Object(o) = value else {
        return Vec::new();
    };
    enum Kind<'gc> {
        Ivars(Vec<(String, Value<'gc>)>),
        Native,
        Leaf,
    }
    let kind = {
        let b = o.borrow();
        match &b.payload {
            ObjectPayload::Instance => {
                let cls = b.class.borrow();
                let mut slots: Vec<(String, usize)> = cls
                    .field_slots
                    .iter()
                    .map(|(n, &s)| (n.clone(), s))
                    .collect();
                slots.sort_by_key(|x| x.1);
                let ivars = slots
                    .into_iter()
                    .filter_map(|(n, s)| b.fields.get(s).map(|v| (n, *v)))
                    .collect();
                Kind::Ivars(ivars)
            }
            ObjectPayload::NativeState(_) => Kind::Native,
            // String / Symbol / Bytes / Block render as a single inline leaf — no children.
            _ => Kind::Leaf,
        }
    };
    match kind {
        Kind::Ivars(ivars) => ivars
            .into_iter()
            .map(|(n, v)| (format!("@{n}"), PpChild::Val(v)))
            .collect(),
        Kind::Native => match value_native_shape(value, &value.class_name()) {
            Some(PpShape::Seq { items, .. }) => items
                .into_iter()
                .enumerate()
                .map(|(i, v)| (format!("[{i}]"), PpChild::Val(v)))
                .collect(),
            Some(PpShape::Entries { entries, .. }) => entries
                .into_iter()
                .map(|(k, _, v)| (k, PpChild::Val(v)))
                .collect(),
            Some(PpShape::Record { fields, .. }) => fields,
            None => Vec::new(),
        },
        Kind::Leaf => Vec::new(),
    }
}

/// Render a native collection via its `PrettyPrint` shape (List/Set/Map), or an opaque
/// `<ClassName>` for any other native object whose internals `.pp` doesn't expose.
fn native_doc<'gc>(value: Value<'gc>, cname: &str, visited: &mut HashSet<usize>) -> Doc {
    let shape = value_native_shape(value, cname);
    match shape {
        Some(PpShape::Seq { open, close, items }) => {
            let docs = items
                .into_iter()
                .map(|v| value_to_doc(v, visited))
                .collect();
            bracket("", open, close, docs)
        }
        Some(PpShape::Entries {
            open,
            close,
            entries,
        }) => {
            let docs = entries
                .into_iter()
                .map(|(k, quoted, v)| {
                    let label = if quoted { quote(&k) } else { k };
                    Doc::Cat(vec![text(label), text(": "), value_to_doc(v, visited)])
                })
                .collect();
            bracket("", open, close, docs)
        }
        Some(PpShape::Record { name, fields }) => {
            // `Name{ field: value … }` — unquoted struct-field labels (unlike `Entries`).
            let docs = fields
                .into_iter()
                .map(|(label, child)| {
                    Doc::Cat(vec![
                        text(format!("{label}: ")),
                        child_to_doc(child, visited),
                    ])
                })
                .collect();
            bracket(name, "{", "}", docs)
        }
        None => non_collection_native_doc(value, cname),
    }
}

/// Render one `Record` field: recurse into a live value, or emit a native-formatted text leaf
/// in its role's color (string-ish leaves are quoted like string literals).
fn child_to_doc<'gc>(child: PpChild<'gc>, visited: &mut HashSet<usize>) -> Doc {
    match child {
        PpChild::Val(v) => value_to_doc(v, visited),
        PpChild::Text(s, PpRole::Str) => styled(Role::Str, text(quote(&s))),
        PpChild::Text(s, PpRole::Number) => styled(Role::Number, text(s)),
    }
}

/// A few non-collection native types have a canonical literal `.pp` form that isn't a `Record`
/// (a regex literal, a method's signature chain); anything else is opaque (`<ClassName>`).
fn non_collection_native_doc<'gc>(value: Value<'gc>, cname: &str) -> Doc {
    match cname {
        // A regex prints as its literal `#/pattern/`.
        "Regex" => value
            .with_native_state::<NativeRegexState, _, _>(|r| format!("#/{}/", r.regex.as_str()))
            .map(|s| styled(Role::Regex, text(s)))
            .unwrap_or_else(|_| text("<Regex>")),
        // A method shows its selector + each multimethod variant's signature.
        "Method" => method_doc(value),
        _ => text(format!("<{cname}>")),
    }
}

/// `Method(<sig> | <sig> | …)` over the multimethod chain; each variant's signature via
/// `introspect::signature` (selector with param types, ` {…}` if guarded, ` (native)` if a
/// native body).
fn method_doc<'gc>(value: Value<'gc>) -> Doc {
    let mut sigs: Vec<String> = Vec::new();
    let mut cur = Some(value);
    while let Some(node) = cur {
        if sigs.len() >= 64 {
            sigs.push("…".to_string()); // guard against a malformed cyclic chain
            break;
        }
        match node
            .with_native_state::<NativeMethodState, _, _>(|m| (variant_signature(m), m.get_next()))
        {
            Ok((sig, next)) => {
                sigs.push(sig);
                cur = next;
            }
            Err(_) => break,
        }
    }
    if sigs.is_empty() {
        return text("<Method>");
    }
    // `Method(` <selector> ` | ` <selector> … `)` — the name and delimiters colored, each
    // variant signature in the selector color.
    let mut parts = vec![
        styled(Role::ClassName, text("Method")),
        styled(Role::Delim, text("(")),
    ];
    for (i, sig) in sigs.into_iter().enumerate() {
        if i > 0 {
            parts.push(text(" | "));
        }
        parts.push(styled(Role::Selector, text(sig)));
    }
    parts.push(styled(Role::Delim, text(")")));
    Doc::Cat(parts)
}

/// One chain node's signature: param types from the user block (with its guard) or from the
/// native method's declared types; an `"Object"` type is normalized to an untyped param.
fn variant_signature(m: &NativeMethodState) -> String {
    let untyped = |t: &String| (t != "Object").then(|| t.clone());
    let (param_types, guarded, native): (Vec<Option<String>>, bool, bool) =
        if let Some(Value::Object(o)) = m.get_block() {
            let b = o.borrow();
            if let ObjectPayload::Block(blk) = &b.payload {
                (
                    blk.template.param_types.iter().map(untyped).collect(),
                    blk.decl_block.is_some(),
                    false,
                )
            } else {
                (Vec::new(), false, false)
            }
        } else {
            let pts = m
                .native_param_types()
                .map(|v| v.iter().map(untyped).collect())
                .unwrap_or_default();
            (pts, false, true)
        };
    introspect::signature(
        &m.selector,
        &MethodVariant {
            param_types,
            ret_type: None,
            guarded,
            native,
            source: None,
            doc: None,
        },
    )
}

/// A `'`-quoted, escaped string literal (`it's` → `'it\'s'`).
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// `Bytes[len] <hex preview>` — never dumps raw bytes (mirrors the Display preview).
fn bytes_repr(preview: &[u8], len: usize) -> String {
    let mut out = format!("Bytes[{len}]");
    for b in preview {
        out.push_str(&format!(" {b:02x}"));
    }
    if len > preview.len() {
        out.push_str(" …");
    }
    out
}

#[cfg(test)]
#[path = "pretty_tests.rs"]
mod tests;
