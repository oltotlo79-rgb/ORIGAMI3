# 伝承折り鶴：数学的クリースパターン一式

## 収録ファイル

- `traditional_crane_complete.fold` — FOLD形式の頂点・線分・山谷・面
- `traditional_crane_coordinates_and_equations.json` — 全座標と全線分の数式
- `traditional_crane_vertices.csv` — 56頂点の座標表
- `traditional_crane_edges_equations.csv` — 114線分の端点・方向・直線式
- `traditional_crane_complete_cp.svg` — ラベルなし展開図
- `traditional_crane_complete_cp_labeled.svg` — 頂点番号付き展開図
- `traditional_crane_complete_cp.png` — 高解像度プレビュー
- `PROMPT_FOR_OTHER_AI_JA.txt` — 他のAIへ渡す描画指示

## 座標系

- 用紙: `[0,1] × [0,1]`
- 原点: 左下
- x軸: 右向き
- y軸: 上向き
- 一辺が `L` の紙: `(X,Y)=(L*x,L*y)`
- SVG: `(X,Y)=(S*x,S*(1-y))`

## 各線分の数学的定義

端点を `P0=(x1,y1)`, `P1=(x2,y2)` とすると、

`P(t)=P0+t(P1-P0), 0<=t<=1`

支持直線は、

`A*x+B*y+C=0`

ただし `A=y1-y2`, `B=x2-x1`, `C=x1*y2-x2*y1` です。

## 山谷記号

- `M`: mountain / 山折り
- `V`: valley / 谷折り
- `B`: boundary / 用紙境界

## 検証値

- 頂点: 56
- 線分: 114
- 面: 59
- 山折り M: 61
- 谷折り V: 41
- 境界 B: 12

## 重要事項

これは折り順図ではなく、完成した鶴を平らに開いたときのクリースパターンです。
SVGでは見やすさのため、山折りを赤実線、谷折りを青破線、境界を黒実線にしています。
