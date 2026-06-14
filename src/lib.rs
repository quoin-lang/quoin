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
    ($args:ident, $variant:ident, $idx:expr) => {
        match $args.get($idx) {
            Some(&Value::$variant(val)) => val,
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
    ($args:ident, $variant:ident, $idx:expr, $err:expr) => {
        match $args.get($idx) {
            Some(&Value::$variant(val)) => val,
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
