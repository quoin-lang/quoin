pub mod ansi_colorizer;
pub mod compiler;
pub mod error;
pub mod instruction;
pub mod parser;
pub mod runtime;
pub mod value;
pub mod vm;

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
                return Err($crate::error::BBError::TypeError {
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
                return Err($crate::error::BBError::TypeError {
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
                return Err($crate::error::BBError::TypeError {
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
    ($args:ident, $variant:ident, $idx:expr) => {
        match $args.get($idx) {
            Some(&Value::Object(obj)) => match &obj.borrow().payload {
                $crate::value::ObjectPayload::$variant(val) => val.clone(),
                _ => {
                    return Err($crate::error::BBError::TypeError {
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
                return Err($crate::error::BBError::TypeError {
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
                    return Err($crate::error::BBError::TypeError {
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
                return Err($crate::error::BBError::TypeError {
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
