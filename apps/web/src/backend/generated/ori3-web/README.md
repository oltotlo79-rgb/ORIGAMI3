# ori3-web WASM binding

These four files are generated artifacts; do not edit them by hand.

```text
ori3_web.js
ori3_web.d.ts
ori3_web_bg.wasm
ori3_web_bg.wasm.d.ts
```

They are generated from this worktree after applying the approved wasm32-only
`web-time 1.1.0`, `getrandom 0.4.3/wasm_js`, and shared clock source.
Use exactly `wasm-bindgen 0.2.126`:

```powershell
$env:CARGO_TARGET_DIR = "<dedicated-target-dir>"
cargo build --locked --offline -p ori3-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir apps/web/src/backend/generated/ori3-web --out-name ori3_web "$env:CARGO_TARGET_DIR\wasm32-unknown-unknown\release\ori3_web.wasm"
```

Regenerate all four files whenever the Rust source changes, then record their
byte sizes and SHA-256 values before treating them as release artifacts. Vite
owns the `.wasm` URL so that production builds get a content hash and follow
the configured base path.
