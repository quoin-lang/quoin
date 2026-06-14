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
                return Err(format!(
                    "Expected {} at argument index {}",
                    stringify!($variant),
                    $idx
                ))
            }
        }
    };
    ($args:ident, $variant:ident, $idx:expr, $err:expr) => {
        match $args.get($idx) {
            Some(&Value::$variant(val)) => val,
            _ => return Err($err.to_string()),
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
                    return Err(format!(
                        "Object at argument index {} is {}, wanted {}",
                        $idx, x, $class_name
                    ))
                }
            },
            _ => return Err(format!("Expected Object at argument index {}", $idx)),
        }
    };
    ($args:ident, $class_name:expr, $idx:expr, $err:expr) => {
        match $args.get($idx) {
            Some(&Value::Object(val)) => match val.borrow().class_name().as_str() {
                $class_name => val,
                x => {
                    return Err(format!(
                        "Object at argument index {} is {}, wanted {}",
                        $idx, x, $class_name
                    ))
                }
            },
            _ => return Err($err.to_string()),
        }
    };
}
