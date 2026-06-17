# Profiling Analysis Summary

An analysis of the profiling data from `profile.json.gz` was performed. The debug symbols for `new-vm` were linked using `dsymutil`, and relative addresses in the profile were mapped and symbolicated using `atos` to locate performance bottlenecks across the VM and parser threads.

---

## 1. Executive Summary

- **Total Execution Split**: The execution is split almost equally between the main VM thread (`new-vm`) and **21 concurrent parser helper threads**.
  - **VM Thread (`new-vm`)**: Total sample weight of **1,296 ms**.
  - **Parser Helper Threads**: Combined sample weight of **1,254 ms** (varying from 1 ms to 337 ms per thread).
- **Core Bottleneck**: The VM thread spends **96.84%** of its time blocked waiting (`Thread::join`) for the parser threads to complete. The actual CPU hotspots are entirely situated within the parser threads, specifically in ANTLR lookahead decision-making (`adaptive_predict`).

---

## 2. Thread-by-Thread Hotspots

### A. Main VM Thread (`new-vm`)
- **Blocked on Joins**: **1,255 ms (96.84%)** of the main thread's time is spent inside `std::sys::thread::unix::Thread::join`. 
- **Symbol Note**: A system library function (at address `0x2af8` inside `libsystem_kernel.dylib` responsible for thread parking/joining) was mapped by `atos` to `<glob::PatternError as core::fmt::Debug>::fmt` due to missing system library debug symbols. However, the call stack confirms the VM is simply sleeping/idle during this time.
- **VM Execution**: The actual VM step execution (`VmState::step_internal`) takes only **~3 ms (0.23%)**, indicating that VM runtime overhead is negligible compared to parsing overhead.

### B. Parser Threads (`parser`)
The 21 parser helper threads exhibit highly uniform bottlenecks. Across all parser threads, **80% to 98%** of the inclusive time is spent within ANTLR's ATN (Alternative Transition Network) lookahead simulation engine:

$$\text{Total Parser Time} \longrightarrow \text{adaptive\_predict (80-98\%)} \longrightarrow \text{closure\_work (70-85\%)}$$

Key hot functions within the parser threads include:

1. **`antlr_rust::parser_atn_simulator::ParserATNSimulator::adaptive_predict`** (Inclusive: **80% – 98%**)
   - This function predicts which grammar rule alternative to choose by simulating ATN transitions.
2. **`antlr_rust::parser_atn_simulator::ParserATNSimulator::closure_work` / `closure_checking_stop_state`** (Inclusive: **70% – 85%**)
   - Performs transitive closure operations over ATN configurations, traversing states to compute reach sets.
3. **`murmur3::murmur3_32::MurmurHasher::write`** (Exclusive: **up to 9%**)
   - Used heavily to hash ATN configurations and prediction contexts for caching.
4. **`<antlr_rust::prediction_context::PredictionContext as PartialEq>::eq`** (Exclusive: **3% – 8%**)
   - Comparing prediction contexts to resolve and merge cache entries.
5. **`<Q as hashbrown::Equivalent<K>>::equivalent`** (Exclusive: **3% – 8%**)
   - Lookup overhead in hash maps/sets when caching prediction results.

---

## 3. Areas for Performance Optimization

Since the hot spots are heavily concentrated in ANTLR's ATN simulator lookahead logic, optimization should focus on parsing efficiency:

### 1. Grammar Tuning (Highest Leverage)
- **Problem**: Large lookahead weights and expensive ATN simulation indicate that the ANTLR grammar has rules requiring deep lookahead to resolve ambiguities.
- **Recommendation**: 
  - Review the parser grammar file (e.g., `.g4` grammar rules).
  - Simplify rules and reduce parser decision points by resolving ambiguities or rewriting complex/left-recursive expressions to be more LL-friendly.
  - Profile the grammar using ANTLR GUI or profiling listeners to find which specific rules trigger the highest lookahead rates.

### 2. Share ATN / DFA Caches Across Threads
- **Problem**: 21 threads are spawned, but if each thread spins up its own isolated parser instance without sharing the `DFASerializer` or decision cache, lookahead states must be recomputed from scratch in every thread.
- **Recommendation**: Ensure that the parser's ATN interpreter shares its DFA prediction cache globally across all parser threads (using thread-safe structures like `Arc` and mutex/rwlocks supported by `antlr-rust`). Sharing the DFA cache lets threads benefit from predictions already computed by other threads.

### 3. Parser Hash Function Optimization
- **Problem**: `MurmurHasher` and hash map equivalence checks are highly visible in the exclusive execution time.
- **Recommendation**: Explore if a faster, non-cryptographic hasher (such as `FxHasher` or `ahash`) can be substituted for lookahead caches, or check if prediction contexts can be simplified to make equality checks faster.

### 4. Alternative Parser Architecture
- **Problem**: The `antlr-rust` runtime has high baseline overhead (allocations, hashing, and lookahead interpretation checks) even for full compiles.
- **Recommendation**: Evaluate migrating the parsing frontend to a more performant alternative:
  - **Tree-sitter**: Utilizes a table-driven C parser engine with thin Rust bindings. While optimized for incremental parsing in IDEs/editors, its baseline full-parse execution is highly efficient and eliminates ANTLR lookahead interpretation overhead.
  - **Rust-native Parser Generators (e.g., LALRPOP, Pest)**: Compile directly to native Rust parsing code, avoiding heavy runtime interpretation.
  - **Hand-written Parser**: A custom recursive-descent parser, which represents the industry standard for production compilers (like `rustc`). It yields maximum performance and clean compiler error reporting, though it requires manual coding.
