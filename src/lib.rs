#![allow(unknown_lints)]

// Modules gated `not(target_arch = "wasm32")` are the OS-bound surface a browser build
// can't carry: the coroutine runner/driver + REPL/DAP front-ends (corosensei, rustyline)
// and the worker subprocess machinery. `codegen` compiles on wasm minus its Cranelift
// translator (see codegen/mod.rs), and `io_backend` gates its smol-backed half inside
// the module. Everything else compiles for wasm as-is.
pub mod ansi_colorizer;
pub mod class_table;
pub mod clock;
pub mod codegen;
pub mod compiler;
pub mod compute;
pub mod coverage;
#[cfg(not(target_arch = "wasm32"))]
pub mod dap;
pub mod debug;
#[cfg(not(target_arch = "wasm32"))]
pub mod debug_cli;
pub mod devirt_ops;
pub mod direct_runner;
pub mod dispatch;
pub mod docs;
pub mod error;
pub mod ext_sdk;
pub mod fiber;
pub mod handle_table;
pub mod highlighter;
pub mod instruction;
pub mod introspect;
pub mod io_backend;
pub mod io_codecs;
pub mod md_html;
pub mod packages;
pub mod parser;
pub mod registry;
#[cfg(not(target_arch = "wasm32"))]
pub mod repl_complete;
#[cfg(not(target_arch = "wasm32"))]
pub mod replay;
#[cfg(not(target_arch = "wasm32"))]
pub mod runner;
pub mod runner_core;
#[cfg(not(target_arch = "wasm32"))]
pub mod runner_pkg;
pub mod runtime;
pub mod stdlib;
pub mod symbol;
pub mod tuning;
pub mod types;
pub mod value;
pub mod vm;
pub mod worker;

#[macro_export]
macro_rules! gc {
    ($mc:ident, $val:expr) => {
        Gc::new($mc, $val)
    };
}

#[macro_export]
macro_rules! gcl {
    ($mc:ident, $val:expr) => {
        Gc::new($mc, RefLock::new($val))
    };
}

#[macro_export]
macro_rules! arg {
    ($args:ident, Class, $idx:expr) => {
        match $args.get($idx) {
            Some(&Value::Class(val)) => val,
            _ => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "Class".to_string(),
                    got: match $args.get($idx) {
                        Some(v) => v.type_name().to_string(),
                        None => "None".to_string(),
                    },
                    msg: format!("Expected Class at index {}", $idx),
                })
            }
        }
    };
    ($args:ident, ClassMeta, $idx:expr) => {
        match $args.get($idx) {
            Some(&Value::ClassMeta(val)) => val,
            _ => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "ClassMeta".to_string(),
                    got: match $args.get($idx) {
                        Some(v) => v.type_name().to_string(),
                        None => "None".to_string(),
                    },
                    msg: format!("Expected ClassMeta at index {}", $idx),
                })
            }
        }
    };
    ($args:ident, Instance, $idx:expr) => {
        match $args.get($idx) {
            Some(&Value::Object(obj)) => obj,
            _ => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "Instance".to_string(),
                    got: match $args.get($idx) {
                        Some(v) => v.type_name().to_string(),
                        None => "None".to_string(),
                    },
                    msg: format!(
                        "Expected {} at argument index {} (got {:?})",
                        "Instance", $idx, $args[$idx],
                    ),
                })
            }
        }
    };
    // Immediate scalar types live directly in `Value` (not in an `Object`
    // payload), so they need their own arms ahead of the generic one below.
    ($args:ident, Int, $idx:expr) => {
        match $args.get($idx) {
            Some(&Value::Int(val)) => val,
            _ => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "Integer".to_string(),
                    got: match $args.get($idx) {
                        Some(v) => v.type_name().to_string(),
                        None => "None".to_string(),
                    },
                    msg: format!("Expected Integer at argument index {}", $idx),
                })
            }
        }
    };
    ($args:ident, Double, $idx:expr) => {
        match $args.get($idx) {
            Some(&Value::Double(val)) => val,
            _ => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "Double".to_string(),
                    got: match $args.get($idx) {
                        Some(v) => v.type_name().to_string(),
                        None => "None".to_string(),
                    },
                    msg: format!("Expected Double at argument index {}", $idx),
                })
            }
        }
    };
    ($args:ident, Bool, $idx:expr) => {
        match $args.get($idx) {
            Some(&Value::Bool(val)) => val,
            _ => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "Boolean".to_string(),
                    got: match $args.get($idx) {
                        Some(v) => v.type_name().to_string(),
                        None => "None".to_string(),
                    },
                    msg: format!("Expected Boolean at argument index {}", $idx),
                })
            }
        }
    };
    ($args:ident, $variant:ident, $idx:expr) => {
        match $args.get($idx) {
            Some(&Value::Object(obj)) => match &obj.borrow().payload {
                $crate::value::ObjectPayload::$variant(val) => val.clone(),
                _ => {
                    return Err($crate::error::QuoinError::TypeError {
                        expected: stringify!($variant).to_string(),
                        got: match $args.get($idx) {
                            Some(v) => v.type_name().to_string(),
                            None => "None".to_string(),
                        },
                        msg: format!(
                            "Expected {} at argument index {} (got {:?})",
                            stringify!($variant),
                            $idx,
                            $args[$idx],
                        ),
                    })
                }
            },
            _ => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: stringify!($variant).to_string(),
                    got: match $args.get($idx) {
                        Some(v) => v.type_name().to_string(),
                        None => "None".to_string(),
                    },
                    msg: format!(
                        "Expected {} at argument index {} (got {:?})",
                        stringify!($variant),
                        $idx,
                        $args[$idx],
                    ),
                })
            }
        }
    };
    ($args:ident, Class, $idx:expr, $err:expr) => {
        match $args.get($idx) {
            Some(&Value::Class(val)) => val,
            _ => return Err($err.into()),
        }
    };
    ($args:ident, ClassMeta, $idx:expr, $err:expr) => {
        match $args.get($idx) {
            Some(&Value::ClassMeta(val)) => val,
            _ => return Err($err.into()),
        }
    };
    ($args:ident, Instance, $idx:expr, $err:expr) => {
        match $args.get($idx) {
            Some(&Value::Object(obj)) => obj,
            _ => return Err($err.into()),
        }
    };
    ($args:ident, $variant:ident, $idx:expr, $err:expr) => {
        match $args.get($idx) {
            Some(&Value::Object(obj)) => match &obj.borrow().payload {
                $crate::value::ObjectPayload::$variant(val) => val.clone(),
                _ => return Err($err.into()),
            },
            _ => return Err($err.into()),
        }
    };
}

#[macro_export]
macro_rules! arg_obj {
    ($args:ident, $class_name:expr, $idx:expr) => {
        match $args.get($idx) {
            Some(&Value::Object(val)) => match val.borrow().class_name().as_str() {
                $class_name => val,
                x => {
                    return Err($crate::error::QuoinError::TypeError {
                        expected: $class_name.to_string(),
                        got: x.to_string(),
                        msg: format!(
                            "Object at argument index {} is {}, wanted {}",
                            $idx, x, $class_name
                        ),
                    })
                }
            },
            _ => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "Object".to_string(),
                    got: match $args.get($idx) {
                        Some(v) => v.type_name().to_string(),
                        None => "None".to_string(),
                    },
                    msg: format!("Expected Object at argument index {}", $idx),
                })
            }
        }
    };
    ($args:ident, $class_name:expr, $idx:expr, $err:expr) => {
        match $args.get($idx) {
            Some(&Value::Object(val)) => match val.borrow().class_name().as_str() {
                $class_name => val,
                x => return Err($err.into()),
            },
            _ => return Err($err.into()),
        }
    };
}

/// Extract a typed value from a native method's `receiver` (its `self`), mirroring
/// `arg!` but for the single receiver value rather than an indexed argument. The
/// immediate scalar types match `Value` directly; the rest match an `ObjectPayload`.
#[macro_export]
macro_rules! recv {
    ($recv:expr, Int) => {
        match $recv {
            Value::Int(val) => val,
            v => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "Integer".to_string(),
                    got: v.type_name().to_string(),
                    msg: "expected an Integer receiver".to_string(),
                })
            }
        }
    };
    ($recv:expr, Double) => {
        match $recv {
            Value::Double(val) => val,
            v => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "Double".to_string(),
                    got: v.type_name().to_string(),
                    msg: "expected a Double receiver".to_string(),
                })
            }
        }
    };
    ($recv:expr, Bool) => {
        match $recv {
            Value::Bool(val) => val,
            v => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "Boolean".to_string(),
                    got: v.type_name().to_string(),
                    msg: "expected a Boolean receiver".to_string(),
                })
            }
        }
    };
    ($recv:expr, Class) => {
        match $recv {
            Value::Class(val) => val,
            v => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: "Class".to_string(),
                    got: v.type_name().to_string(),
                    msg: "expected a Class receiver".to_string(),
                })
            }
        }
    };
    ($recv:expr, $variant:ident) => {
        match $recv {
            Value::Object(obj) => match &obj.borrow().payload {
                $crate::value::ObjectPayload::$variant(val) => val.clone(),
                _ => {
                    return Err($crate::error::QuoinError::TypeError {
                        expected: stringify!($variant).to_string(),
                        got: $recv.type_name().to_string(),
                        msg: format!("expected a {} receiver", stringify!($variant)),
                    })
                }
            },
            v => {
                return Err($crate::error::QuoinError::TypeError {
                    expected: stringify!($variant).to_string(),
                    got: v.type_name().to_string(),
                    msg: format!("expected a {} receiver", stringify!($variant)),
                })
            }
        }
    };
}
