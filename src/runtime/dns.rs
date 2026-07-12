//! The `DNS` class — name resolution over the backend's `Resolve`/`ResolveReverse`
//! ops (getaddrinfo/getnameinfo on the blocking pool: the resolver `Connect` has
//! always used internally, exposed). Class methods only; lookups park the task,
//! not the scheduler. Record-type queries (TXT/MX/SRV) would need a real DNS
//! client dependency — deferred, rationale in QUOIN_TODO.

use crate::arg;
use crate::error::QuoinError;
use crate::io_backend::{IoRequest, IoResult};
use crate::value::{NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;

/// One `Resolve` round trip: the deduplicated IP strings for `host`.
fn resolve<'gc>(vm: &mut VmState<'gc>, host: &str, who: &str) -> Result<Vec<String>, QuoinError> {
    match vm.await_io(IoRequest::Resolve {
        host: host.to_string(),
    })? {
        IoResult::Resolved(ips) => Ok(ips),
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(QuoinError::Other(format!(
            "{who}: unexpected I/O result {other:?}"
        ))),
    }
}

fn ips_to_list<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>, ips: Vec<String>) -> Value<'gc> {
    let values = ips.into_iter().map(|ip| vm.new_string(mc, ip)).collect();
    vm.new_list(mc, values)
}

pub fn build_dns_class() -> NativeClassBuilder {
    NativeClassBuilder::new("DNS", Some("Object"))
        .construct_with("DNS is all class methods (use DNS.resolve:)")
        .class_doc(
            "Name resolution — the system resolver (getaddrinfo, the same one \
             `TcpSocket.connect:` uses), exposed. Lookups run on the blocking pool and \
             park the task, not the scheduler.\n\n\
             ```\n\
             DNS.resolve:'localhost'      \"* a List of IP strings, e.g. #( 127.0.0.1 ::1 )\n\
             DNS.reverse:'127.0.0.1'      \"* a hostname, or nil when unmapped\n\
             ```\n\n\
             Forward lookups answer A + AAAA addresses in resolver order (deduplicated); \
             a name that doesn't resolve throws a catchable IoError. Record-type queries \
             (TXT/MX/SRV) are not the system resolver's job and are deliberately absent.",
        )
        .typed_class_method("resolve:", &["String"], |vm, mc, _r, args| {
            let host = arg!(args, String, 0).to_string();
            let ips = resolve(vm, &host, "DNS.resolve:")?;
            Ok(ips_to_list(vm, mc, ips))
        })
        .doc(
            "Every address for `host` — IPv4 and IPv6, as Strings, in resolver order \
             (deduplicated). A name that doesn't resolve throws a catchable IoError.",
        )
        .typed_class_method("resolve4:", &["String"], |vm, mc, _r, args| {
            let host = arg!(args, String, 0).to_string();
            let ips = resolve(vm, &host, "DNS.resolve4:")?;
            let v4 = ips.into_iter().filter(|ip| !ip.contains(':')).collect();
            Ok(ips_to_list(vm, mc, v4))
        })
        .doc("`resolve:`, IPv4 addresses only (possibly empty).")
        .typed_class_method("resolve6:", &["String"], |vm, mc, _r, args| {
            let host = arg!(args, String, 0).to_string();
            let ips = resolve(vm, &host, "DNS.resolve6:")?;
            let v6 = ips.into_iter().filter(|ip| ip.contains(':')).collect();
            Ok(ips_to_list(vm, mc, v6))
        })
        .doc("`resolve:`, IPv6 addresses only (possibly empty).")
        .typed_class_method("reverse:", &["String"], |vm, mc, _r, args| {
            let addr = arg!(args, String, 0).to_string();
            match vm.await_io(IoRequest::ResolveReverse { addr })? {
                IoResult::Resolved(names) => Ok(match names.into_iter().next() {
                    Some(name) => vm.new_string(mc, name),
                    None => vm.new_nil(mc),
                }),
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                other => Err(QuoinError::Other(format!(
                    "DNS.reverse:: unexpected I/O result {other:?}"
                ))),
            }
        })
        .doc(
            "The hostname the IP reverse-resolves to (a PTR lookup via getnameinfo), or \
             nil when the address has no mapping — that is an answer, not an error. A \
             string that isn't an IP address throws a ValueError-kinded IoError.",
        )
}
