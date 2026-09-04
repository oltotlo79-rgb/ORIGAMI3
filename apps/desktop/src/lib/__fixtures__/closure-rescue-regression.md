`closure-rescue-regression.json` は、2026-09-03 に退役した台本の展開図で、折り鶴の標本ではない。
閉包救済（closure rescue）が紙の食い込み検出の有無に依らず効くことの回帰検査の入力としてだけ使う。
この入力は単発 solve では紙を閉じられず、そのことが検査の前提なので 1 バイトも変えない。
読んでいるのは `apps/desktop/src-tauri/src/store.rs` の
`crane_closure_rescue_is_independent_of_contact_detection` 1 箇所だけである。
折り鶴の正本は `crates/ori3-layers/tests/fixtures/traditional-crane/traditional-crane-cp.ori3`。
