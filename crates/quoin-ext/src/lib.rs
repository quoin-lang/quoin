//! quoin-ext — the **extension-side** SDK for out-of-process Quoin extensions
//! (Tier 1 of the extension architecture; see `docs/FUTURE_EXT_ARCH.md`).
//!
//! An extension is a separate process the Quoin VM spawns and talks to over a unix
//! domain socket. This crate is the thin per-language client an extension links
//! against — it is **not** linked into the VM. (The VM-side host API is the
//! separate in-process `ext_sdk` surface.)
//!
//! ## Wire protocol
//!
//! Messages are length-prefixed frames: a little-endian `u32` length followed by that many
//! payload bytes. The payload is a FlatBuffers `Message` union (schema + codec in the shared
//! `quoin-ext-proto` crate). A `Call` carries typed handle args — host-value handles via
//! [`Host::handles`] (a block is one of these) and ext-side resource ids via [`Host::resources`].
//! The handler may issue **re-entrant host-ops** through the [`Host`] client — `make_string`,
//! `handle_to_string`, `retain`, `release`, `call_method` (send a Quoin message to a handle),
//! `invoke_block` (run a host block over a batch in one round-trip), and the host-reach ops
//! `get_global` (reach a host class/global), `make_value` (build any host value from a
//! [`DataValue`]), and `read_handle` (project a handle to a [`DataValue`]) — each a synchronous
//! round-trip the host services while parked on the reply. It returns a [`Reply`]: a scalar string,
//! an ext-side **resource** (reaped via [`Host::releases`] on a later call), an [`ArrowArray`] (the
//! bulk data plane, copy-through), a structured [`DataValue`] tree, or a live host [`Handle`].
//! Structured args arrive via [`Host::data`]; bulk columns via [`Host::arrays`]. Host values the
//! extension holds are opaque [`Handle`]s indexing a GC-rooted table on the host (§2).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::os::unix::net::{UnixListener, UnixStream};

pub use quoin_ext_proto::{ArrowArray, ArrowDType, DataValue};
use quoin_ext_proto::{ClassDecl, Msg};

/// An opaque reference to a host value, as seen by the extension. Default lifetime is
/// call-local (auto-released when the originating call returns); promote it with
/// [`Host::retain`] to hold it across calls.
pub type Handle = u64;

/// Read one length-prefixed frame. `Ok(None)` on a clean EOF (peer closed between
/// frames); `Err` on a truncated frame or other I/O error.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

/// Write `payload` as one length-prefixed frame and flush.
pub fn write_frame(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// The host-callback client handed to a request handler for the duration of one `Call`.
///
/// Each method issues a re-entrant host-op (a `Msg` frame the host services mid-call) and
/// blocks on the matching `HostOpReturn`. It borrows the connection mutably, so host-ops are
/// strictly serialized within the call — exactly the request/response ping-pong the host's
/// service loop expects.
pub struct Host<'a> {
    stream: &'a mut UnixStream,
    /// Host-value handle args passed on this `Call` (a block is one of these), in order.
    handles: Vec<Handle>,
    /// Ext-side resource ids passed back as args on this `Call`, in order.
    resources: Vec<u64>,
    /// Ext-side resource ids the host has dropped; the handler should free them from its own
    /// registry (typically at the top of the call). Empty unless this extension hands out resources.
    releases: Vec<u64>,
    /// Bulk `Array` columns passed as args on this `Call`, in order (the data plane).
    arrays: Vec<ArrowArray>,
    /// The structured-value payload passed via `call:with:data:`, if any.
    data: Option<DataValue>,
}

impl<'a> Host<'a> {
    /// The host-value handle args for this call (see [`Host::invoke_block`] to run a block one).
    pub fn handles(&self) -> &[Handle] {
        &self.handles
    }

    /// The ext-side resource ids passed back as args for this call (look them up in your registry).
    pub fn resources(&self) -> &[u64] {
        &self.resources
    }

    /// Ext-side resource ids the host has dropped — free these from your registry.
    pub fn releases(&self) -> &[u64] {
        &self.releases
    }

    /// The bulk `Array` columns passed as args on this call (read `dtype`/`data` and crunch them).
    pub fn arrays(&self) -> &[ArrowArray] {
        &self.arrays
    }

    /// The structured-value payload passed via `call:with:data:`, as a `DataValue` tree, if any.
    pub fn data(&self) -> Option<&DataValue> {
        self.data.as_ref()
    }

    /// Make a host `String` value and return a (call-local) handle to it.
    pub fn make_string(&mut self, s: &str) -> io::Result<Handle> {
        let (handle, _) = self.host_op(&Msg::MakeString {
            value: s.to_string(),
        })?;
        Ok(handle)
    }

    /// Read a `String`-typed handle back into a Rust string.
    pub fn handle_to_string(&mut self, handle: Handle) -> io::Result<String> {
        let (_, str) = self.host_op(&Msg::HandleToString { handle })?;
        str.ok_or_else(|| invalid_data("HandleToString reply carried no string"))
    }

    /// Promote a call-local handle to retained (global), so it survives past this call.
    pub fn retain(&mut self, handle: Handle) -> io::Result<()> {
        self.host_op(&Msg::Retain { handle }).map(|_| ())
    }

    /// Release retained handles (batched).
    pub fn release(&mut self, handles: &[Handle]) -> io::Result<()> {
        self.host_op(&Msg::Release {
            handles: handles.to_vec(),
        })
        .map(|_| ())
    }

    /// Send the Quoin message `selector` to the value behind `receiver`, with the values
    /// behind `args` as the arguments, and return a (call-local) handle to the result. The
    /// host performs a real method dispatch; a host-reported error (bad handle, wrong arity,
    /// or a raise during the send) surfaces as an `io::Error`.
    pub fn call_method(
        &mut self,
        receiver: Handle,
        selector: &str,
        args: &[Handle],
    ) -> io::Result<Handle> {
        let (handle, _) = self.host_op(&Msg::CallMethodOnHandle {
            receiver,
            selector: selector.to_string(),
            args: args.to_vec(),
        })?;
        Ok(handle)
    }

    /// Invoke the host block `block` once per tuple in `batches`, in a single round-trip, and
    /// return one result handle per tuple (in order). The host runs the block N times locally;
    /// `batches` of length 1 is a single call. A bad handle or a raise surfaces as an `io::Error`.
    pub fn invoke_block(
        &mut self,
        block: Handle,
        batches: &[Vec<Handle>],
    ) -> io::Result<Vec<Handle>> {
        write_frame(
            self.stream,
            &quoin_ext_proto::encode(&Msg::InvokeBlock {
                block,
                batches: batches.to_vec(),
            }),
        )?;
        let frame = read_frame(self.stream)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "host closed mid-host-op")
        })?;
        match quoin_ext_proto::decode_envelope(&frame).map_err(invalid_data)? {
            Msg::InvokeBlockReturn { results, error } => match error {
                Some(e) => Err(io::Error::other(e)),
                None => Ok(results),
            },
            other => Err(invalid_data(format!(
                "expected InvokeBlockReturn, got {other:?}"
            ))),
        }
    }

    /// Apply a host block (a `Handle` from a method argument — see [`Arg::handle`]) to each input,
    /// in one batched round-trip, returning one result per input. Each input is made into a host
    /// value, the block is invoked once per input, and each result is read back as a [`DataValue`].
    /// The unary mapping form (`v map: { |x| … }`); for richer call shapes use [`Host::invoke_block`]
    /// directly.
    pub fn apply_block(
        &mut self,
        block: Handle,
        inputs: &[DataValue],
    ) -> io::Result<Vec<DataValue>> {
        let batches: Vec<Vec<Handle>> = inputs
            .iter()
            .map(|d| Ok(vec![self.make_value(d.clone())?]))
            .collect::<io::Result<_>>()?;
        let results = self.invoke_block(block, &batches)?;
        results.iter().map(|h| self.read_handle(*h)).collect()
    }

    /// Resolve a name in the host's globals (a class is a class-valued global), returning a handle
    /// to its value — so the extension can reach and drive host classes/globals (e.g.
    /// `call_method(get_global("Array"), "ofFloats:", [list])`). Unbound name -> `io::Error`.
    pub fn get_global(&mut self, name: &str) -> io::Result<Handle> {
        let (handle, _) = self.host_op(&Msg::GetGlobal {
            name: name.to_string(),
        })?;
        Ok(handle)
    }

    /// Construct any host value from a [`DataValue`] tree, returning a handle to it (for building
    /// non-string method arguments). The general form of [`Host::make_string`].
    pub fn make_value(&mut self, value: DataValue) -> io::Result<Handle> {
        let (handle, _) = self.host_op(&Msg::MakeValue { value })?;
        Ok(handle)
    }

    /// Project the value behind `handle` to a [`DataValue`] tree — inspect any handle as native
    /// data (the general form of [`Host::handle_to_string`]).
    pub fn read_handle(&mut self, handle: Handle) -> io::Result<DataValue> {
        write_frame(
            self.stream,
            &quoin_ext_proto::encode(&Msg::ReadHandle { handle }),
        )?;
        let frame = read_frame(self.stream)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "host closed mid-host-op")
        })?;
        match quoin_ext_proto::decode_envelope(&frame).map_err(invalid_data)? {
            Msg::ReadHandleReturn { value, error } => match error {
                Some(e) => Err(io::Error::other(e)),
                None => Ok(value),
            },
            other => Err(invalid_data(format!(
                "expected ReadHandleReturn, got {other:?}"
            ))),
        }
    }

    /// Send one host-op and await its `HostOpReturn`, surfacing a host-reported error as an
    /// `io::Error`. Returns the reply's `(handle, str)` payload (either may be unset).
    fn host_op(&mut self, msg: &Msg) -> io::Result<(Handle, Option<String>)> {
        write_frame(self.stream, &quoin_ext_proto::encode(msg))?;
        let frame = read_frame(self.stream)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "host closed mid-host-op")
        })?;
        match quoin_ext_proto::decode_envelope(&frame).map_err(invalid_data)? {
            Msg::HostOpReturn { handle, str, error } => match error {
                Some(e) => Err(io::Error::other(e)),
                None => Ok((handle, str)),
            },
            other => Err(invalid_data(format!(
                "expected HostOpReturn, got {other:?}"
            ))),
        }
    }
}

fn invalid_data(e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// What a call handler returns: a scalar string, an ext-side resource id, a bulk `Array` (the data
/// plane), or a structured value (materialized as a nested Quoin Value). A `String`/`&str` converts
/// to `Reply::Scalar`, and a `DataValue` to `Reply::Data`, so handlers need little ceremony.
pub enum Reply {
    Scalar(String),
    Resource(u64),
    Array(ArrowArray),
    Data(DataValue),
    /// Return a live host value the extension holds (a handle from `get_global`/`make_value`/
    /// `call_method`); the host resolves it to the value and returns it to the caller.
    Handle(Handle),
}

impl From<String> for Reply {
    fn from(s: String) -> Self {
        Reply::Scalar(s)
    }
}

impl From<&str> for Reply {
    fn from(s: &str) -> Self {
        Reply::Scalar(s.to_string())
    }
}

impl From<DataValue> for Reply {
    fn from(d: DataValue) -> Self {
        Reply::Data(d)
    }
}

/// Bind a unix socket at `path`, accept one host connection, and serve requests until the host
/// disconnects. Each `Call` frame invokes `handler(host, op, arg)`; the handler may use `host` to
/// read the call's handle/resource/array args and issue re-entrant host-ops, then returns a
/// [`Reply`] (scalar / resource / array) sent back as `CallReturn` / `CallReturnResource` /
/// `CallReturnArray`.
///
/// Blocking and single-connection by design: the extension is its own process, and the VM holds
/// exactly one connection to it. Returns once the host disconnects.
pub fn serve<R: Into<Reply>>(
    path: &str,
    mut handler: impl FnMut(&mut Host, &str, &str) -> R,
) -> io::Result<()> {
    let listener = UnixListener::bind(path)?;
    let (mut stream, _addr) = listener.accept()?;
    while let Some(frame) = read_frame(&mut stream)? {
        match quoin_ext_proto::decode_envelope(&frame).map_err(invalid_data)? {
            // A generic-handler extension provides no classes (Phase 3): reply with an empty
            // manifest so it stays backward-compatible under the host's spawn-time `GetManifest`.
            Msg::GetManifest => {
                write_frame(
                    &mut stream,
                    &quoin_ext_proto::encode(&Msg::ManifestReturn {
                        classes: Vec::new(),
                    }),
                )?;
            }
            Msg::Call {
                op,
                arg,
                handles,
                resources,
                releases,
                arrays,
                data,
                // The generic `call:with:` handler doesn't use the extension-backed-class fields.
                class_name: _,
                recv: _,
                method_args: _,
            } => {
                // `host` borrows the stream for the call's re-entrant host-ops; the borrow ends
                // before we write the terminal reply on the same stream.
                let reply: Reply = {
                    let mut host = Host {
                        stream: &mut stream,
                        handles,
                        resources,
                        releases,
                        arrays,
                        data,
                    };
                    handler(&mut host, &op, &arg).into()
                };
                write_frame(&mut stream, &quoin_ext_proto::encode(&reply_to_msg(reply)))?;
            }
            other => {
                return Err(invalid_data(format!(
                    "extension serve: expected a Call to begin a conversation, got {other:?}"
                )));
            }
        }
    }
    Ok(())
}

/// Encode a handler's [`Reply`] as the matching terminal `CallReturn*` frame.
fn reply_to_msg(reply: Reply) -> Msg {
    match reply {
        Reply::Scalar(result) => Msg::CallReturn { result },
        Reply::Resource(resource) => Msg::CallReturnResource {
            resource,
            class_name: String::new(),
        },
        Reply::Array(array) => Msg::CallReturnArray { array },
        Reply::Data(value) => Msg::CallReturnData { value },
        Reply::Handle(handle) => Msg::CallReturnHandle { handle },
    }
}

// ---------------------------------------------------------------------------
// Extension-backed classes (Phase 3): the SDK owns the object table.
//
// An extension provides one or more Quoin classes by registering plain Rust types. The host
// installs a real Quoin class whose method sends dispatch over the socket; the SDK keeps the
// instances in its own table keyed by an opaque id (the resource id the host holds), so writing
// an extension-backed class feels like writing an ordinary type — no handle plumbing in the
// method bodies, and instances are freed automatically when the host drops them.
// ---------------------------------------------------------------------------

/// One resolved method argument handed to a handler (Phase 3 — richer args). `Data` is a decoded
/// value; `Object` is another of *this extension's* live instances (resolved from the object table —
/// downcast with [`Arg::object`]); `Handle` is a host-value handle for a block or other host object
/// the handler drives via [`Host::apply_block`] / the [`Host`] callbacks.
pub enum Arg<'a> {
    Data(DataValue),
    Object(&'a dyn Any),
    Handle(Handle),
}

impl<'a> Arg<'a> {
    /// The decoded value, if this argument is plain data.
    pub fn data(&self) -> Option<&DataValue> {
        match self {
            Arg::Data(d) => Some(d),
            _ => None,
        }
    }

    /// The live instance behind an ext-object argument, downcast to `T` (the type it was built as).
    pub fn object<T: 'static>(&self) -> Option<&T> {
        match self {
            Arg::Object(o) => o.downcast_ref::<T>(),
            _ => None,
        }
    }

    /// The host-value handle behind a block / non-data argument (drive it via [`Host::apply_block`]).
    pub fn handle(&self) -> Option<Handle> {
        match self {
            Arg::Handle(h) => Some(*h),
            _ => None,
        }
    }
}

/// Erased per-selector handlers (the concrete `T` is captured at registration and downcast here).
type CtorFn = Box<dyn Fn(&mut Host, &[Arg]) -> io::Result<Box<dyn Any>>>;
type MethodFn = Box<dyn Fn(&mut dyn Any, &mut Host, &[Arg]) -> io::Result<Reply>>;
type MakesFn = Box<dyn Fn(&mut dyn Any, &mut Host, &[Arg]) -> io::Result<Box<dyn Any>>>;

/// One registered class's erased handler tables, keyed by selector.
struct ClassReg {
    name: String,
    constructors: HashMap<String, CtorFn>,
    methods: HashMap<String, MethodFn>,
    makes: HashMap<String, MakesFn>,
}

/// Downcast a table entry to the concrete type the handler was registered with.
fn downcast<T: 'static>(obj: &mut dyn Any) -> io::Result<&mut T> {
    obj.downcast_mut::<T>()
        .ok_or_else(|| invalid_data("extension instance is not of the expected type"))
}

/// Configures one extension-backed class, backed by the Rust type `T`. Class-side `constructor`s
/// build a `T`; instance-side `method`s read/mutate it and return a [`Reply`]; `makes` produce a
/// new instance (of this or any registered class). Method arguments arrive as [`Arg`]s — data,
/// another of the extension's live instances (`Arg::object`), or a host block (`Arg::handle`).
pub struct ClassBuilder<T> {
    name: String,
    constructors: HashMap<String, CtorFn>,
    methods: HashMap<String, MethodFn>,
    makes: HashMap<String, MakesFn>,
    _marker: PhantomData<fn() -> T>,
}

impl<T: 'static> ClassBuilder<T> {
    /// A class-side constructor: `Class sel: …` builds a new `T` (stored in the object table); the
    /// Quoin caller receives an instance.
    pub fn constructor(
        &mut self,
        selector: &str,
        f: impl Fn(&mut Host, &[Arg]) -> T + 'static,
    ) -> &mut Self {
        self.constructors.insert(
            selector.to_string(),
            Box::new(move |host, args| Ok(Box::new(f(host, args)) as Box<dyn Any>)),
        );
        self
    }

    /// An instance-side method returning a value (a scalar string or structured [`DataValue`]).
    pub fn method<R: Into<Reply>>(
        &mut self,
        selector: &str,
        f: impl Fn(&mut T, &mut Host, &[Arg]) -> R + 'static,
    ) -> &mut Self {
        self.methods.insert(
            selector.to_string(),
            Box::new(move |obj, host, args| Ok(f(downcast::<T>(obj)?, host, args).into())),
        );
        self
    }

    /// An instance-side method that yields a new instance — of this class (`scale:` / `clone`) or
    /// of *any* registered class (`Matrix.row:` -> `Vector`, a cross-class return). The returned
    /// type's registered class is recovered by `TypeId` at dispatch, so the host wraps it correctly.
    pub fn makes<U: 'static>(
        &mut self,
        selector: &str,
        f: impl Fn(&mut T, &mut Host, &[Arg]) -> U + 'static,
    ) -> &mut Self {
        self.makes.insert(
            selector.to_string(),
            Box::new(move |obj, host, args| {
                Ok(Box::new(f(downcast::<T>(obj)?, host, args)) as Box<dyn Any>)
            }),
        );
        self
    }

    fn into_reg(self) -> ClassReg {
        ClassReg {
            name: self.name,
            constructors: self.constructors,
            methods: self.methods,
            makes: self.makes,
        }
    }
}

/// A class-providing extension (Phase 3). Register classes with [`Extension::class`], then
/// [`Extension::serve`]. The SDK owns the instances (a `u64 -> Box<dyn Any>` table); the host holds
/// only opaque ids, and dropped instances are reaped from the table automatically.
#[derive(Default)]
pub struct Extension {
    classes: Vec<ClassReg>,
    /// Maps each registered Rust type to its Quoin class name, so an instance returned by a method
    /// (of this class or another) is wrapped as the right class on the host — cross-class returns.
    type_names: HashMap<TypeId, String>,
}

impl Extension {
    pub fn new() -> Self {
        Self {
            classes: Vec::new(),
            type_names: HashMap::new(),
        }
    }

    /// Register a class named `name` backed by the Rust type `T`; `build` configures its
    /// constructors and methods.
    pub fn class<T: 'static>(
        &mut self,
        name: &str,
        build: impl FnOnce(&mut ClassBuilder<T>),
    ) -> &mut Self {
        self.type_names.insert(TypeId::of::<T>(), name.to_string());
        let mut cb = ClassBuilder {
            name: name.to_string(),
            constructors: HashMap::new(),
            methods: HashMap::new(),
            makes: HashMap::new(),
            _marker: PhantomData,
        };
        build(&mut cb);
        self.classes.push(cb.into_reg());
        self
    }

    /// Bind a unix socket at `path`, accept the host connection, and serve until it disconnects:
    /// answer the spawn-time `GetManifest` from the registered classes, and route each method
    /// `Call` to its handler — materializing returned instances into the object table.
    pub fn serve(&self, path: &str) -> io::Result<()> {
        let listener = UnixListener::bind(path)?;
        let (mut stream, _addr) = listener.accept()?;
        let mut table = ObjectTable::default();
        while let Some(frame) = read_frame(&mut stream)? {
            match quoin_ext_proto::decode_envelope(&frame).map_err(invalid_data)? {
                Msg::GetManifest => {
                    write_frame(&mut stream, &quoin_ext_proto::encode(&self.manifest()))?;
                }
                Msg::Call {
                    op,
                    class_name,
                    recv,
                    method_args,
                    releases,
                    ..
                } => {
                    // The host batches dropped instances onto `releases`; free them from the table.
                    for rid in &releases {
                        table.take(*rid);
                    }
                    let reply = self.dispatch(
                        &mut stream,
                        &mut table,
                        &class_name,
                        &op,
                        recv,
                        &method_args,
                    )?;
                    write_frame(&mut stream, &quoin_ext_proto::encode(&reply))?;
                }
                other => {
                    return Err(invalid_data(format!(
                        "extension serve: expected a Call or GetManifest, got {other:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// The registered Quoin class name for a freshly built instance, recovered from its concrete
    /// type — so a method returning an instance of *any* registered class is wrapped correctly
    /// (cross-class returns). Empty if the type isn't registered (defensively, an `ExtResource`).
    fn class_name_of(&self, obj: &dyn Any) -> String {
        self.type_names
            .get(&obj.type_id())
            .cloned()
            .unwrap_or_default()
    }

    /// The `ManifestReturn` describing every registered class.
    fn manifest(&self) -> Msg {
        let classes = self
            .classes
            .iter()
            .map(|c| ClassDecl {
                name: c.name.clone(),
                instance_selectors: c.methods.keys().chain(c.makes.keys()).cloned().collect(),
                class_selectors: c.constructors.keys().cloned().collect(),
            })
            .collect();
        Msg::ManifestReturn { classes }
    }

    /// Route one method `Call` to its handler and produce the terminal reply frame.
    fn dispatch(
        &self,
        stream: &mut UnixStream,
        table: &mut ObjectTable,
        class_name: &str,
        op: &str,
        recv: u64,
        method_args: &[quoin_ext_proto::Arg],
    ) -> io::Result<Msg> {
        let class = self
            .classes
            .iter()
            .find(|c| c.name == class_name)
            .ok_or_else(|| invalid_data(format!("no extension-backed class '{class_name}'")))?;
        let mut host = Host {
            stream,
            handles: Vec::new(),
            resources: Vec::new(),
            releases: Vec::new(),
            arrays: Vec::new(),
            data: None,
        };
        if recv == 0 {
            // Class-side: a constructor builds a new instance.
            let ctor = class.constructors.get(op).ok_or_else(|| {
                invalid_data(format!("no constructor '{op}' on class '{class_name}'"))
            })?;
            let obj = {
                let args = resolve_args(method_args, table)?;
                ctor(&mut host, &args)?
            };
            let class_name = self.class_name_of(&*obj);
            Ok(Msg::CallReturnResource {
                resource: table.insert(obj),
                class_name,
            })
        } else if let Some(method) = class.methods.get(op) {
            // Take the receiver out of the table so its `&mut` can't alias an ext-instance argument
            // resolved from the same table, then put it back under its id.
            let mut recv_box = table
                .take(recv)
                .ok_or_else(|| invalid_data(format!("no live instance {recv}")))?;
            let reply = {
                let args = resolve_args(method_args, table)?;
                method(recv_box.as_mut(), &mut host, &args)?
            };
            table.reinsert(recv, recv_box);
            Ok(reply_to_msg(reply))
        } else if let Some(makes) = class.makes.get(op) {
            let mut recv_box = table
                .take(recv)
                .ok_or_else(|| invalid_data(format!("no live instance {recv}")))?;
            let new_obj = {
                let args = resolve_args(method_args, table)?;
                makes(recv_box.as_mut(), &mut host, &args)?
            };
            table.reinsert(recv, recv_box);
            let class_name = self.class_name_of(&*new_obj);
            Ok(Msg::CallReturnResource {
                resource: table.insert(new_obj),
                class_name,
            })
        } else {
            Err(invalid_data(format!(
                "no method '{op}' on class '{class_name}'"
            )))
        }
    }
}

/// Resolve the wire arguments to the handler-facing [`Arg`]s: data passes through, an ext-instance
/// id is looked up to a live object (a shared borrow of the table), and a handle passes through.
fn resolve_args<'t>(
    method_args: &[quoin_ext_proto::Arg],
    table: &'t ObjectTable,
) -> io::Result<Vec<Arg<'t>>> {
    method_args
        .iter()
        .map(|a| match a {
            quoin_ext_proto::Arg::Data(d) => Ok(Arg::Data(d.clone())),
            quoin_ext_proto::Arg::Resource(id) => table
                .get(*id)
                .map(Arg::Object)
                .ok_or_else(|| invalid_data(format!("argument references no live instance {id}"))),
            quoin_ext_proto::Arg::Handle(h) => Ok(Arg::Handle(*h)),
        })
        .collect()
}

/// The SDK-owned instance table (Phase 3): live instances keyed by an opaque id — the resource id
/// the host holds for each. Ids start at 1, so `recv == 0` unambiguously means a class-side send.
#[derive(Default)]
struct ObjectTable {
    objects: HashMap<u64, Box<dyn Any>>,
    next_id: u64,
}

impl ObjectTable {
    /// Store an instance under a fresh id and return it.
    fn insert(&mut self, obj: Box<dyn Any>) -> u64 {
        self.next_id += 1;
        self.objects.insert(self.next_id, obj);
        self.next_id
    }

    /// A shared borrow of the live instance for `id`, or `None` if it isn't (or no longer) live.
    /// Shared so several args (and the receiver, once it's been `take`n out) can be resolved at once.
    fn get(&self, id: u64) -> Option<&dyn Any> {
        self.objects.get(&id).map(|b| &**b)
    }

    /// Remove and return the instance for `id` (e.g. the receiver of an instance method, or one the
    /// host has dropped), or `None` if it isn't live.
    fn take(&mut self, id: u64) -> Option<Box<dyn Any>> {
        self.objects.remove(&id)
    }

    /// Put a previously-`take`n instance back under its id.
    fn reinsert(&mut self, id: u64, obj: Box<dyn Any>) {
        self.objects.insert(id, obj);
    }
}
