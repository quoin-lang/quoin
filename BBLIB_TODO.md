# BuildingBlocks Runtime & Library TODO List

This document outlines the language features, compiler updates, and VM modifications required to execute the BuildingBlocks standard library (`bblib`) files and test suites.

## Misc
- [ ] Get rid of `Value::Native`, it's only used by the global funcs and those are only used for testing.
  - In the BB language itself all methods are attached to a class.
- [ ] Support checking `assertMeetsRequirements:` in calls to `mix:`/`can:`.

## Bugs/Odd Behavior

## 1. Class & Method Definition Semantics
- [x] **Class Creation (`<-` operator)**:
  - Implement AST compilation for `IDENTIFIER <- BLOCK` expressions. This should define a new `Value::Class` and store it in `globals`.
  - The block body must be executed with the new Class object as the default receiver (`self`).
  - Declare instance variables using the block's parameters (e.g. `| @x @y |` inside the class definition block).
- [x] **Class/Instance Extension (`<--` operator)**:
  - Implement `IDENTIFIER <-- BLOCK` behavior. This adds new methods to either a Class meta-object or a specific object instance (singleton/eigenclass methods).
- [x] **Method Definitions (`->`) and Overrides (`-->`)**:
  - `SELECTOR -> BLOCK`: Define a new method on the current subject. Raise an error if it already exists.
  - `SELECTOR --> BLOCK`: Override an existing method. Raise an error if it does not exist.
  - Support normalize selectors for operator symbols (e.g., mapping `#'-'` to `-`, `#'+:'` to `+:`).
- [x] **Class Meta-object (`.meta`)**:
  - Implement a `.meta` method on `Class` to retrieve/define class-side (static/constructor) methods.

## 2. Object Instantiation & Instance Variables
- [x] **Instantiation Block Syntax (`.new:`)**:
  - Support `Class.new: { ... }`.
  - The block must run in the context of the newly created instance. Instance variable names (without the `@` prefix) are bound as local variables or directly assignable inside the block to initialize fields.
- [x] **Instance Variables (`@variable`)**:
  - Support reading/writing instance variables via the `@` prefix in method definitions.
  - Map field names to their storage on the `Object` struct.

## 3. Mixins & Multiple Inheritance
- [x] **Mixin Registration (`.mix:`)**:
  - Implement `.mix:CLASS` to copy or link behaviors from a mixed-in class.
- [x] **Mixin Method Resolution**:
  - Update `lookup_method` in the VM to search through mixed-in classes (depth-first or breadth-first) before checking parent classes.

## 4. Advanced Method Dispatch (Multimethods / Argument Types)
- [ ] **Typed Block Arguments**:
  - Support parameter type checking inside block headers: `| name:Type |`.
- [ ] **Method Overloading**:
  - Resolve messages by matching both the selector name *and* matching the types of the arguments passed at runtime.
  - E.g., `split: -> { |pat:String| ... }` vs `split: --> { |p:Regex| ... }` must dispatch correctly depending on whether the argument is a `String` or a `Regex`.

## 5. Non-Local Returns (`^^` operator)
- [x] **Method-level returns (`^^`)**:
  - Implement the `^^` return operator.
  - When a block executes `^^ value`, it must return from the enclosing method that created the block.
  - This requires closures (`Block`) to hold a reference to their creator's stack frame, and the VM to unwind frames up to that context.

## 6. Exception Handling & unwinding (`catch:` and `throw`)
- [ ] **Throwing Exceptions**:
  - Support `.throw` and `.throw:` on objects.
- [ ] **Catches**:
  - Support `.catch:{ ... }` blocks.
  - The VM must unwind execution frames back to the nearest enclosing catch block when an exception is thrown.

## 7. Namespaces
- [x] **Namespaced Globals**:
  - Support namespaced identifiers like `[IO]Stdout` or `[IO]Folder`.
  - The compiler and VM must parse, store, and look up namespaced globals.

## 8. Built-in Core Library Extensions
- [x] **Boolean & Nil Logic**:
  - Implement `if:`, `else:`, `if:else:`, and `not` purely as methods on the `true`, `false`, and `nil` objects in `bootstrap.b`, rather than using VM-level jump instructions.
- [x] **IO Library**:
  - Implement native classes under `[IO]` namespace: `[IO]Stdout`, `[IO]Stderr`, `[IO]Handle`, and `[IO]Folder`.
- [ ] **System Utilities**:
  - `Timer.time: { ... }`: Computes elapsed time in milliseconds.
  - `Runtime.evalFile: filename`: Loads, compiles, and evaluates a file.
  - `Object.s` overrides: Overriding `s` string representation when converting objects to strings for printing.
- [x] **Native State Support**:
  - Implement native classes holding arbitrary Rust state inside VM objects, following [native_rust_state_plan.md](file:///Users/damon/code/new-vm/native_rust_state_plan.md).
