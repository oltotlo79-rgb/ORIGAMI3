# ORIGAMI3 デスクトップアプリ

折り紙設計アプリ ORIGAMI3 の画面部分(Tauri 2 + React + TypeScript + Vite)。

## 起動方法(開発時)

このフォルダ(`apps/desktop`)で実行する:

```powershell
npm install        # 初回のみ
npm run tauri dev  # アプリのウィンドウが起動する
```

## 検査

リポジトリ全体の一括検査はリポジトリのルートで実行する:

```powershell
powershell -File scripts/check.ps1
```

(計算部分のテスト・静的検査、画面部分のビルド・文法検査の4つをまとめて実行する)
