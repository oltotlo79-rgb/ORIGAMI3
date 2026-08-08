# ORIGAMI3

展開図を描きながら3Dモデルで1折りずつ折り紙を折り、新作を設計するためのデスクトップアプリ。

- 骨格(角の数・長さ・膨らみ)を指定して展開図を自動提案
- グリッド + 作図補助による展開図編集
- ヒンジごとの角度指定による剛体折り3D表示(3D上に折り線を描いて重ね折り→展開図へ自動反映)
- 折り手順の記録・自動再生、折り図PDF書き出し

前身の [ORIGAMI2](../ORIGAMI2) の反省(厳密演算による肥大・UIの複雑化)を踏まえ、「実用上正しい」を優先して新規開発する。

## Windows版の入手と起動

### インストーラー版（推奨）

1. [GitHub Releases](https://github.com/oltotlo79-rgb/ORIGAMI3/releases) から、使いたいバージョンの `ORIGAMI3_x.x.x_setup.exe` をダウンロードする。
2. ダウンロードしたファイルをダブルクリックし、画面の案内に従ってインストールする。管理者権限は不要。
3. インストール完了後にORIGAMI3を起動する。自動で開かなかった場合は、スタートメニューの「ORIGAMI3」から起動する。

### ポータブル版（インストール不要）

同じReleaseにある `ORIGAMI3_x.x.x_portable.exe` をダウンロードし、その1ファイルをダブルクリックすると起動できる。フロントエンド資産はexeに埋め込まれている。Windows 10/11に標準搭載されている Microsoft Edge WebView2 Runtime が必要。

### 初回起動時の注意

現在の配布物にはコード署名がないため、Windows SmartScreenの警告が表示される場合がある。その場合は、内容とダウンロード元を確認してから「詳細情報」→「実行」を選ぶ。

Windows Smart App Controlが有効な環境では、署名のないexeが完全にブロックされ、「実行」を選べないことがある。この場合は配布物側にコード署名が必要であり、インストーラー版・ポータブル版とも起動できない可能性がある。

### 開発者向け: リリース作成

1. `Cargo.toml` の `workspace.package.version`、`apps/desktop/package.json` と `apps/desktop/package-lock.json`、`apps/desktop/src-tauri/tauri.conf.json` の `version` を同じ値に更新してコミットする。
2. `scripts/check.ps1` を実行し、全5検査が通ることを確認する。
3. バージョンと同じタグを作成してpushする（例: `git tag v0.1.0`、`git push origin v0.1.0`）。
4. GitHub ActionsがWindows版をビルドし、NSISインストーラー、MSI、ポータブルexe、取扱説明書PDFをGitHub Releasesへ添付する。タグと設定上のバージョンが一致しない場合、ビルドは失敗する。

既存タグのリリースを再ビルドする場合は、GitHub Actionsの「Release Windows」から手動実行し、そのタグ名（例: `v0.1.0`）を入力する。

## ドキュメント

- [ORIGAMI3取扱説明書](docs/manual/ORIGAMI3取扱説明書.pdf)
- [要件定義書](docs/requirements-definition.md)
- [実装ロードマップ](docs/implementation-roadmap.md)

## 技術構成

Tauri 2 / React / TypeScript / Three.js / Rust計算コア

## ライセンス

[MIT License](LICENSE)
