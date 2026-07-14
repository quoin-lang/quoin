//! `Greeter` — a complete extension-backed class, top to bottom.
//!
//! An extension is a separate process the Quoin VM spawns; it serves method calls over a
//! unix domain socket whose path arrives as the first command-line argument. This example
//! provides one Quoin class exercising each registration kind:
//!
//! - `constructor` — `Greeter named: 'World'` builds an instance (and shows a *recoverable*
//!   error: a rejected argument raises a catchable Quoin error, the extension keeps running).
//! - `method` — `g greet` returns a value from a live instance.
//! - `makes` — `g louder` returns a *new* instance (of this or any registered class).
//! - `class_method` — `Greeter defaultName` returns a value from the class side.
//! - resources-in-data — `Greeter crowd: #('Ada' 'Grace')` returns a `List` of live
//!   instances via [`Value::instance`].
//!
//! Try it from the repository root:
//!
//! ```text
//! cargo build -p quoin-ext --example greeter
//! cargo run -- -e "
//!     Extension.spawn:'target/debug/examples/greeter';
//!     var g = Greeter.named:'World';
//!     g.greet.print;                                       \"* Hello, World!
//!     g.louder.greet.print;                                \"* Hello, World!!
//!     Greeter.defaultName.print;                           \"* World
//!     (Greeter.crowd:#( 'Ada' 'Grace' )).each:{ |p| p.greet.print };
//!     ({ Greeter.named:'' }.catch:{ |e| e.message }).print \"* a greeter needs a name
//! "
//! ```
//!
//! The e2e-tested twin of this example is `tests/fixtures/ext_vector.rs` (driven by
//! `tests/extension.rs`); this file is the reading copy. To ship an extension as a
//! `use`-able package instead of a hand-spawned binary, see `docs/internal/EXT_PACKAGING.md` —
//! the sibling `Quernfile.qn` packages this example that way (`quern` in this directory).

use quoin_ext::{Arg, DataValue, Extension, Value};

/// A plain Rust type. The SDK owns the instances (an id-keyed object table); the host holds
/// opaque ids and method sends arrive already routed to the right instance.
struct Greeter {
    name: String,
    punctuation: String,
}

impl Greeter {
    fn new(name: &str) -> Greeter {
        Greeter {
            name: name.to_string(),
            punctuation: "!".to_string(),
        }
    }

    fn greeting(&self) -> String {
        format!("Hello, {}{}", self.name, self.punctuation)
    }
}

/// Read a string data argument (`args[i]` carrying a Quoin String).
fn str_arg(args: &[Arg], i: usize) -> String {
    match args.get(i).and_then(|a| a.data()) {
        Some(DataValue::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn main() {
    // The VM passes the socket path to rendezvous on as the first argument.
    let path = std::env::args()
        .nth(1)
        .expect("usage: greeter <socket-path>");

    let mut ext = Extension::new();
    ext.class::<Greeter>("Greeter", |c| {
        // Class-side constructor: `Greeter named: 'World'` -> a live instance. Returning
        // `Err` is a RECOVERABLE error: the host raises a catchable Quoin error and this
        // process keeps serving (crashing, by contrast, kills only this extension — the
        // VM survives either way).
        c.constructor("named:", |_host, args| {
            let name = str_arg(args, 0);
            if name.is_empty() {
                return Err("a greeter needs a name".into());
            }
            Ok(Greeter::new(&name))
        });

        // Instance method returning a value: any `Into<Reply>` — a String replies as a
        // Quoin String, a `DataValue`/`Value` tree as structured data.
        c.method("greet", |g, _host, _args| Ok(g.greeting()));

        // `makes` — a method whose result is a NEW instance (here of the same class; a
        // different registered class works too, recovered by its Rust type).
        c.makes("louder", |g, _host, _args| {
            Ok(Greeter {
                name: g.name.clone(),
                punctuation: format!("{}!", g.punctuation),
            })
        });

        // Class-side selector returning a value rather than a constructed instance.
        c.class_method("defaultName", |_host, _args| Ok("World"));

        // Resources-in-data: a structured return tree whose leaves include NEW live
        // instances (`Value::instance`) — here a `List` of Greeters. Lowering is atomic:
        // instances enter the object table only if the whole tree validates.
        c.class_method("crowd:", |_host, args| {
            let Some(DataValue::List(names)) = args.first().and_then(|a| a.data()) else {
                return Err("crowd: expects a list of names".into());
            };
            let greeters = names
                .iter()
                .map(|n| match n {
                    DataValue::Str(s) => Ok(Value::instance(Greeter::new(s))),
                    _ => Err("crowd: expects a list of names"),
                })
                .collect::<Result<Vec<Value>, _>>()?;
            Ok(Value::List(greeters))
        });
    });

    ext.serve(&path).expect("greeter serve loop");
}
