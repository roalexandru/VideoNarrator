# Rust Async Safety Patterns for Tauri

Reference guide for identifying and fixing async safety issues in Tauri applications. Read this before auditing Phase 2.

## Blocking I/O in Async Context

### The Problem

Tauri command handlers run on the Tokio async runtime. Any blocking call (synchronous file I/O, synchronous process spawning, CPU-intensive computation) blocks one of Tokio's limited worker threads. With enough blocked threads, the entire UI freezes.

### Identifying Blocking Calls

Search for these patterns inside `async fn`:

| Blocking Pattern | Async Replacement |
|-----------------|-------------------|
| `std::fs::read()` | `tokio::fs::read()` |
| `std::fs::write()` | `tokio::fs::write()` |
| `std::fs::create_dir_all()` | `tokio::fs::create_dir_all()` |
| `std::fs::remove_file()` | `tokio::fs::remove_file()` |
| `std::fs::remove_dir_all()` | `tokio::fs::remove_dir_all()` |
| `std::fs::read_dir()` | `tokio::fs::read_dir()` |
| `std::fs::metadata()` | `tokio::fs::metadata()` |
| `std::fs::copy()` | `tokio::fs::copy()` |
| `std::fs::rename()` | `tokio::fs::rename()` |
| `std::fs::read_to_string()` | `tokio::fs::read_to_string()` |
| `std::process::Command` | `tokio::process::Command` |
| `std::thread::sleep()` | `tokio::time::sleep()` |

### When to Use `spawn_blocking`

Use `tokio::task::spawn_blocking` when:
- The blocking operation cannot be replaced with an async equivalent (e.g., third-party crate with synchronous API)
- CPU-intensive computation (hashing, image processing, PDF parsing)
- Multiple sequential blocking calls that are logically grouped

```rust
// Before: blocks Tokio thread
let hash = blake3::hash(&data);

// After: offloads to blocking thread pool
let hash = tokio::task::spawn_blocking(move || {
    blake3::hash(&data)
}).await?;
```

### Exception: Quick Metadata Checks

A single `path.exists()` or `path.is_file()` check is borderline — it's a single syscall that typically completes in microseconds. In hot loops, replace it. In one-off checks at the start of a command, it's acceptable to leave as-is with a comment explaining why.

## Mutex Safety

### std::sync::Mutex vs tokio::sync::Mutex

**Rule:** If a lock is held across an `.await` point, it MUST be `tokio::sync::Mutex`. If the lock is acquired and released without any `.await` in between, `std::sync::Mutex` is fine (and actually slightly faster).

```rust
// DANGEROUS: std::sync::Mutex held across .await
let guard = self.data.lock().unwrap();
let result = some_async_call(&guard).await;  // Deadlock risk!
drop(guard);

// SAFE: Extract data, drop lock, then await
let data = {
    let guard = self.data.lock().unwrap();
    guard.clone()  // or extract what you need
};
let result = some_async_call(&data).await;

// SAFE: Use tokio::sync::Mutex when you must hold across .await
let guard = self.data.lock().await;
let result = some_async_call(&guard).await;
drop(guard);
```

### Lock Poisoning

`std::sync::Mutex::lock()` returns a `Result` that is `Err` if a thread panicked while holding the lock. In a Tauri app, panicking with `unwrap()` cascades the failure.

```rust
// DANGEROUS
let guard = mutex.lock().unwrap();

// SAFE: Recover from poisoning (the data may still be valid)
let guard = mutex.lock().unwrap_or_else(|e| e.into_inner());

// SAFE: Propagate as error
let guard = mutex.lock().map_err(|_| NarratorError::InternalError("lock poisoned".into()))?;
```

### Deadlock Prevention

- Always acquire multiple locks in the same order across all code paths
- Keep lock scopes as small as possible — extract data, drop lock, then process
- Never call external functions or async code while holding a lock
- In Tauri commands: lock `State`, extract/clone what you need, `drop()` the guard, then do async work

## Channel Patterns

### Backpressure

Unbounded channels (`tokio::sync::mpsc::unbounded_channel`) can cause OOM if the producer outpaces the consumer. Use bounded channels with appropriate capacity:

```rust
// DANGEROUS: unbounded, producer can overwhelm consumer
let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

// SAFE: bounded with backpressure
let (tx, rx) = tokio::sync::mpsc::channel(32);
```

For Tauri `Channel<T>` (IPC progress reporting), the channel is managed by Tauri — sends that fail (frontend not listening) should use `.ok()` to discard the error, not `.unwrap()`.

### Select and Cancellation Safety

When using `tokio::select!`, the non-selected branch is dropped. Ensure that dropping a future mid-execution doesn't leak resources:

```rust
tokio::select! {
    result = long_operation() => handle(result),
    _ = cancel_signal.recv() => {
        // long_operation's future is dropped here
        // Ensure it doesn't hold file handles, temp files, etc.
        cleanup();
    }
}
```

## Drop Order and Async Cleanup

### The Problem

Rust's `Drop` trait is synchronous — you cannot `.await` inside `drop()`. If an async resource needs cleanup (closing a connection, deleting temp files), the cleanup must happen before the value is dropped.

```rust
// WRONG: Can't await in Drop
impl Drop for TempFileManager {
    fn drop(&mut self) {
        tokio::fs::remove_file(&self.path).await; // Compile error!
    }
}

// RIGHT: Explicit async cleanup before drop
async fn process_video(path: &Path) -> Result<(), NarratorError> {
    let temp = create_temp_file().await?;
    let result = do_work(&temp).await;
    // Clean up before temp goes out of scope
    tokio::fs::remove_file(&temp.path).await.ok();
    result
}
```

### Spawned Task Cleanup

Spawned tasks (`tokio::spawn`) run independently. If the parent is cancelled, spawned tasks continue running. Use `JoinHandle::abort()` or a cancellation token to stop them:

```rust
let handle = tokio::spawn(async move {
    // Long-running work
});

// If we need to cancel:
handle.abort();
// Or check the cancel flag:
if cancel_flag.load(Ordering::Relaxed) {
    handle.abort();
}
```

## Error Handling in Async Context

### Panics in Spawned Tasks

A panic in `tokio::spawn` does NOT propagate to the parent — it's silently caught. The `JoinHandle` returns `Err(JoinError)` which contains the panic payload.

```rust
let handle = tokio::spawn(async {
    panic!("oops"); // This won't crash the app, but the task silently fails
});

match handle.await {
    Ok(result) => result,
    Err(e) if e.is_panic() => {
        tracing::error!("Task panicked: {:?}", e);
        return Err(NarratorError::InternalError("internal task failed".into()));
    }
    Err(e) => {
        tracing::error!("Task cancelled: {:?}", e);
        return Err(NarratorError::Cancelled);
    }
}
```

### The `?` Operator in Spawned Tasks

`?` inside `tokio::spawn` returns from the spawned closure, not the outer function. The error is captured in the `JoinHandle`:

```rust
let result = tokio::spawn(async {
    let data = tokio::fs::read("file.txt").await?; // Returns from closure
    Ok::<_, std::io::Error>(data)
}).await??; // First ? unwraps JoinError, second ? unwraps io::Error
```

## Performance Considerations

### Tokio Worker Threads

By default, Tokio creates one worker thread per CPU core. A Tauri app typically doesn't need this many. Consider configuring:

```rust
// In main.rs or lib.rs, if using #[tokio::main]:
#[tokio::main(worker_threads = 4)]
```

However, Tauri manages the runtime internally — only configure this if you're building a custom runtime.

### spawn_blocking Thread Pool

The `spawn_blocking` pool is separate from the async worker pool and defaults to 512 threads. It auto-scales. Each `spawn_blocking` call may create a new OS thread, so avoid calling it in tight loops — batch work instead:

```rust
// BAD: Spawns N blocking tasks for N files
for file in files {
    tokio::task::spawn_blocking(move || process(file)).await?;
}

// GOOD: Batch into one blocking task
let results = tokio::task::spawn_blocking(move || {
    files.iter().map(|f| process(f)).collect::<Vec<_>>()
}).await?;
```
