//! The native-class registry: every Rust-backed class registered on a fresh `VmState`.
//!
//! Extracted from `runner.rs` so the wasm build (which compiles the runner out — it is
//! coroutine-based) shares the exact same builtin set as the native runner modes
//! (run/test/benchmark/repl) and the two can't drift. OS-bound classes (sockets, DNS,
//! process, folders…) register on wasm too: they bottom out in the I/O backend, which
//! reports them `Unsupported` there — a catchable Quoin error, not a missing class.
//! Even `Fiber` registers (the stdlib mentions it); on wasm its `new:` raises.

use crate::runtime::{
    array, async_rt, big_decimal, big_integer, block, boolean, bytes, channel, civil, class,
    codecs, crypto, csv_fmt, date_time, dns, double, duration, extension, fiber as fiber_class,
    http, ids, instant, integer, io, json, lang_ast, list, map, math, method, msgpack, nil, object,
    os, process, random_access, regex, runtime, set, sockets, span, streams, string, symbol, task,
    term, time_zone, timer, timestamp, toml_fmt, vm_stats, yaml,
};
use crate::vm::VmState;
use gc_arena::Mutation;

pub fn register_builtins<'gc>(mc: &Mutation<'gc>, vm: &mut VmState<'gc>) {
    vm.register_native_class(mc, object::build_object_class());
    vm.register_native_class(mc, class::build_class_class());
    vm.register_native_class(mc, boolean::build_boolean_class());
    vm.register_native_class(mc, block::build_block_class());
    vm.register_native_class(mc, bytes::build_bytes_class());
    vm.register_native_class(mc, codecs::build_base64_class());
    vm.register_native_class(mc, codecs::build_hex_class());
    vm.register_native_class(mc, crypto::build_crypto_digest_class());
    vm.register_native_class(mc, crypto::build_crypto_hmac_class());
    vm.register_native_class(mc, crypto::build_crypto_random_class());
    vm.register_native_class(mc, json::build_json_class());
    vm.register_native_class(mc, lang_ast::build_lang_parser_class());
    vm.register_native_class(mc, lang_ast::build_lang_node_class());
    vm.register_native_class(mc, msgpack::build_message_pack_class());
    vm.register_native_class(mc, csv_fmt::build_csv_class());
    vm.register_native_class(mc, ids::build_uuid_class());
    vm.register_native_class(mc, ids::build_ulid_class());
    vm.register_native_class(mc, channel::build_channel_class());
    vm.register_native_class(mc, toml_fmt::build_toml_class());
    vm.register_native_class(mc, yaml::build_yaml_class());
    vm.register_native_class(mc, dns::build_dns_class());
    vm.register_native_class(mc, sockets::build_tcp_socket_class());
    vm.register_native_class(mc, sockets::build_tls_socket_class());
    vm.register_native_class(mc, sockets::build_tcp_listener_class());
    vm.register_native_class(mc, http::build_http_parser_class());
    vm.register_native_class(mc, streams::build_byte_stream_class());
    vm.register_native_class(mc, random_access::build_random_access_class());
    vm.register_native_class(mc, streams::build_string_stream_class());
    vm.register_native_class(mc, os::build_os_path_class());
    vm.register_native_class(mc, os::build_os_env_class());
    vm.register_native_class(mc, process::build_process_class());
    vm.register_native_class(mc, io::build_io_folder_class());
    vm.register_native_class(mc, io::build_io_file_class());
    vm.register_native_class(mc, io::build_io_handle_class());
    vm.register_native_class(mc, io::build_io_stdin_class());
    vm.register_native_class(mc, vm_stats::build_vm_stats_class());
    vm.register_native_class(mc, crate::runtime::worker::build_worker_class());
    vm.register_native_class(mc, crate::runtime::worker::build_host_block_class());
    vm.register_native_class(mc, list::build_list_class());
    vm.register_native_class(mc, set::build_set_class());
    vm.register_native_class(mc, array::build_array_class());
    vm.register_native_class(mc, runtime::build_runtime_class());
    vm.register_native_class(mc, async_rt::build_async_class());
    vm.register_native_class(mc, task::build_task_class());
    vm.register_native_class(mc, method::build_method_class());
    vm.register_native_class(mc, timer::build_timer_class());
    vm.register_native_class(mc, double::build_double_class());
    vm.register_native_class(mc, integer::build_integer_class());
    vm.register_native_class(mc, math::build_math_class());
    vm.register_native_class(mc, big_decimal::build_big_decimal_class());
    vm.register_native_class(mc, big_integer::build_big_integer_class());
    vm.register_native_class(mc, duration::build_duration_class());
    vm.register_native_class(mc, instant::build_instant_class());
    vm.register_native_class(mc, time_zone::build_time_zone_class());
    vm.register_native_class(mc, timestamp::build_timestamp_class());
    vm.register_native_class(mc, date_time::build_date_time_class());
    vm.register_native_class(mc, civil::build_date_class());
    vm.register_native_class(mc, civil::build_time_class());
    vm.register_native_class(mc, span::build_span_class());
    vm.register_native_class(mc, string::build_string_class());
    vm.register_native_class(mc, symbol::build_symbol_class());
    vm.register_native_class(mc, nil::build_nil_class());
    vm.register_native_class(mc, map::build_map_class());
    vm.register_native_class(mc, map::build_key_value_pair_class());
    vm.register_native_class(mc, regex::build_regex_class());
    vm.register_native_class(mc, regex::build_match_class());
    vm.register_native_class(mc, fiber_class::build_fiber_class());
    vm.register_native_class(mc, extension::build_extension_class());
    vm.register_native_class(mc, term::build_term_class());
}
