# ORIGAMI3 Web

Cloudflare Pagesへ公開するブラウザ版の入口です。画面本体は`apps/desktop`と同じReactソースを使い、Web専用の案内と組み立て設定だけをこのディレクトリに置きます。

## ローカルでの組み立て

先に`apps/desktop`の依存部品を用意した状態で、このディレクトリから次を実行します。

```powershell
npm.cmd run build
```

成果物は`apps/web/dist`へ出ます。取扱説明書PDFはPagesへ複製せず、GitHub Releasesの最新版へリンクします。

## 公開の考え方

実際のworkflowはまだ有効化しません。案は`scratchpad/web-app/proposed/.github/workflows/web.yml`にあり、CIが利用できる状態になってから`.github/workflows/web.yml`へ移します。

案はバージョンタグ（`v*`）のpushだけで動きます。Windowsデスクトップ版のリリースと同じタグ・同じcommitからWeb版を組み立てるため、画面側の更新が同時に反映されます。Cloudflare側では、次の値を用意します。

- Secret `CLOUDFLARE_API_TOKEN`: Pagesへ公開できるAPIトークン
- Secret `CLOUDFLARE_ACCOUNT_ID`: CloudflareのアカウントID
- Repository variable `CLOUDFLARE_PAGES_PROJECT`: 公開先のPagesプロジェクト名

## Cloudflare Pages無料枠に収まる根拠

| 制限 | この構成での扱い |
|---|---|
| 500 build/月 | mainへの通常pushでは組み立てず、デスクトップ版と同じ`v*`タグだけを対象にする。Web build回数は月のリリースタグ数と同じで、月500タグ以下を運用条件とする。 |
| 同時build 1件 | GitHub Actionsの`concurrency`を1グループに固定し、先行中の公開を取り消さず直列に待たせる。 |
| 20,000ファイル | 公開直前に`apps/web/dist`の全ファイルを数え、20,000件超なら公開を止める。2026-08-27の実測は21件。 |
| 25 MiB/ファイル | 公開直前に26,214,400 bytes超の全ファイルを検出し、1件でもあれば公開を止める。現在の最大は369,696 bytes。 |

取扱説明書PDFは28,843,335 bytes（約27.5 MiB）で25 MiBを超えるため、Web成果物には入れません。「はい」を選んだときにGitHub Releasesの最新版`ORIGAMI3.pdf`を直接開きます。これにより、現在の成果物は21ファイル、合計3,583,229 bytes、25 MiB超0件です。
