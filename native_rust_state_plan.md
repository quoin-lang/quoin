# Design Plan: Holding Native Rust State in VM Objects

This design plan outlines how we can extend the VM to allow native classes (e.g. `[IO]File`, `[Network]Socket`) to hold arbitrary Rust state in their instances while exposing those instances to the runtime as regular objects.

## 1. Objectives

- Allow native classes to store Rust-specific handles (e.g. `std::fs::File`, `std::net::TcpStream`) inside VM objects.
- Ensure integration with the `gc_arena` garbage collector (`Collect<'gc>` trait).
- Support dynamic downcasting of native state inside native method callbacks.
- Maintain safety, preventing memory leaks and ensuring resources are properly closed when GC-collected.

## 2. Core Architecture

We will achieve this by extending the `ObjectPayload` enum with a new variant that holds an opaque, GC-traceable box of native state.

### A. The `AnyCollect` Trait

We define a trait `AnyCollect<'gc>` that combines `Collect<'gc>` and downcasting capabilities to `std::any::Any`:

```rust
pub trait AnyCollect<'gc>: gc_arena::Collect<'gc> {
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}
```

Since we need a trait object type `Box<dyn AnyCollect<'gc>>` to be collected by `gc_arena`, we implement `Collect<'gc>` for the trait object box:

```rust
unsafe impl<'gc> gc_arena::Collect<'gc> for Box<dyn AnyCollect<'gc>> {
    fn trace(&self, cc: &gc_arena::Collection<'gc>) {
        // Delegate tracing to the underlying concrete type
        self.as_ref().trace(cc);
    }
}
```

### B. Extending `ObjectPayload`

We add the `NativeState` variant to `ObjectPayload` in `src/value.rs`:

```diff
 #[derive(Clone, Copy, Collect)]
 #[collect(no_drop)]
 pub enum ObjectPayload<'gc> {
     Nil,
     Bool(bool),
     Int(i64),
     Double(f64),
     String(Gc<'gc, String>),
     List(Gc<'gc, RefLock<Vec<Value<'gc>>>>),
     Dict(Gc<'gc, RefLock<HashMap<String, Value<'gc>>>>),
     Regex(Gc<'gc, GcRegex>),
     Block(Gc<'gc, Block<'gc>>),
     Native(NativeFunc),
     Instance,
+    NativeState(Gc<'gc, RefLock<Box<dyn AnyCollect<'gc>>>>),
 }
```

### C. Blanket Implementation for `'static` State

For native state that does not contain GC-managed pointers (e.g., `std::fs::File`, custom buffers, etc.), we provide a blanket implementation to make integration automatic:

```rust
pub struct OpaqueState<T>(pub T);

unsafe impl<'gc, T: 'static> gc_arena::Collect<'gc> for OpaqueState<T> {
    const NEEDS_TRACE: bool = false; // 'static holds no Gc pointers
}

impl<'gc, T: 'static> AnyCollect<'gc> for OpaqueState<T> {
    fn as_any(&self) -> &dyn std::any::Any {
        &self.0
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        &mut self.0
    }
}
```

---

## 3. Creating and Using Native State

### A. Factory helper in `VmState`

To instantiate an object containing native state, we add a helper to `VmState`:

```rust
impl<'gc> VmState<'gc> {
    pub fn new_native_state<T: AnyCollect<'gc> + 'static>(
        &self,
        mc: &gc_arena::Mutation<'gc>,
        class_obj: gc_arena::Gc<'gc, gc_arena::lock::RefLock<Class<'gc>>>,
        state: T,
    ) -> Value<'gc> {
        let payload = ObjectPayload::NativeState(gcl!(mc, Box::new(state) as Box<dyn AnyCollect<'gc>>));
        let obj = gcl!(
            mc,
            Object {
                id: GcUlid(ulid::Ulid::new()),
                class: class_obj,
                fields: HashMap::new(),
                payload,
            }
        );
        Value::Object(obj)
    }
}
```

### B. Downcasting Helper for Native Methods

To easily retrieve the Rust state in native methods, we define a helper or macro:

```rust
pub fn downcast_state<'a, 'gc, T: 'static>(val: Value<'gc>) -> Result<std::cell::Ref<'a, T>, String> {
    if let Value::Object(obj) = val {
        let borrowed = obj.borrow();
        if let ObjectPayload::NativeState(state_cell) = &borrowed.payload {
            let state_ref = state_cell.borrow();
            // Downcast to target type
            if let Some(concrete) = state_ref.as_any().downcast_ref::<T>() {
                // Return borrowed reference
                todo!("Map standard Ref to T");
            }
        }
    }
    Err("Not a native state of the requested type".to_string())
}
```

---

## 4. Practical Example: Namespaced `[IO]File`

Here is a conceptual example of how `[IO]File` would be registered and used:

```rust
use std::fs::File;
use std::io::{Read, Write};

// Define FileWrapper as 'static state
pub struct FileWrapper {
    pub file: File,
}

pub fn register_io_file_class(vm: &mut VmState<'gc>, mc: &Mutation<'gc>) {
    let file_builder = NativeClassBuilder::new("[IO]File", Some("Object"))
        .class_method("open:", |vm, mc, args| {
            let path_val = arg!(args, String, 1);
            let file = File::open(&path_val).map_err(|e| BBError::Other(e.to_string()))?;
            let class_obj = vm.get_builtin_class("[IO]File");
            
            // Wrap in OpaqueState and instantiate
            let state = OpaqueState(FileWrapper { file });
            Ok(vm.new_native_state(mc, class_obj, state))
        })
        .instance_method("readAll", |vm, mc, args| {
            let receiver = args[0];
            // Retrieve file state
            let mut borrowed_state = get_mut_state::<FileWrapper>(receiver)?;
            let mut contents = String::new();
            borrowed_state.file.read_to_string(&mut contents).map_err(|e| BBError::Other(e.to_string()))?;
            Ok(vm.new_string(mc, contents))
        });
        
    vm.register_native_class(mc, file_builder);
}
```

## 5. Alternative Options Considered

1. **Primitive File Descriptor (FD) payload**: Instead of arbitrary Rust state, we could just add `FD(RawFd)` to `ObjectPayload`.
   - *Pros*: Extremely simple.
   - *Cons*: Limited to files/sockets. Cannot support complex Rust structs, database client handles, or external library states.
2. **Untyped pointer payload (`*mut c_void`)**:
   - *Pros*: Flexible.
   - *Cons*: Highly unsafe, prone to memory safety bugs, bypasses Rust's type system and borrow checker.
