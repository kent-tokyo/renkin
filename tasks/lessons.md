# RENKIN - Lessons Learned

## L1: chematic の `run_reactants` は全 mapped atom を BFS シードにする

**問題**: SMIRKS で複数の product テンプレートがある場合、各 product の BFS は `global_map` 全体（すべてのマップ済み原子）をシードにする。product 1 のフラグメントに product 2 の原子が混入する。

**症状**: `run_reactants` の戻り値に `.` を含む canonical SMILES が現れる（例: `"c1c(C(=O)O)cccc1.C[C](O)=[O]"`）。

**対処**: 各 product Molecule の canonical SMILES を `.` で split し、独立した分子として処理。`standardize(remove_explicit_h: true)` を適用。

---

## L2: chematic の canonical SMILES は atom insertion order 依存（未修正: v0.4.10 時点）

**問題**: `CC(=O)O`（酢酸）と `OC(C)=O`（同じ酢酸）で canonical SMILES が一致しない。Morgan ランクのタイブレークが atom の挿入順序に依存。

**症状**: building block 辞書に `CC(=O)O` を登録しても、SMIRKS が産出した `OC(C)=O` をビルディングブロックと認識できない。

**対処**: 文字列比較ではなく **VF2 サブ構造マッチ（`parse_smarts` + `find_matches`）** による完全グラフ同型判定に切り替え。原子数・結合数のHashMap事前フィルタで性能確保。

---

## L3: 芳香族 C は大文字 C の SMIRKS ではマッチしない

**問題**: `[C:1][C:2]>>...` は脂肪族 C-C にしかマッチしない。芳香族 C（lowercase `c`）は別扱い。

**対処**: 芳香族向け専用ルールを別途定義する（`aryl_amine_retro` 等）。

---

## L4: BFS リークで「環なし芳香族鎖」フラグメントが生成される

**問題**: 芳香族分子に SMIRKS を適用すると、開鎖の芳香族フラグメント（例: `cccc(N)c`）が産出される。

**症状**: `cccc(N)c` はパース成功するが building block ではなく、routes が見つからない。

**対処**: フラグメント分割後に「芳香族原子を持つが aromatic_ring_count == 0」の分子を除外。

```rust
let has_aromatic = canonical_smiles(&std_mol).chars().any(|c| matches!(c, 'c'|'n'|'o'|'s'|'p'));
if has_aromatic && aromatic_ring_count(&std_mol) == 0 { return None; }
```

---

## L5: ターゲット自身が precursor に出現する「縮退ルート」

**問題**: カルボン酸（-COOH）に ester_cleavage を適用すると「A → A + water」のような経路が生成される。

**対処**: precursor セットにターゲット SMILES 自身が含まれる場合はスキップ。

```rust
if precursors.iter().any(|p| p.smiles == target_smi) { continue; }
```

---

## L6: building block はコードではなくファイルで管理する

**注意**: `DEFAULT_BUILDING_BLOCKS`（`lib.rs` 内）はコード内フォールバック。実際の探索では `data/building_blocks.smi` が優先読み込みされる。

**対処**: 新しい building block は必ず `data/building_blocks.smi` に追加する。

---

## L7: ターゲット自体が building block のとき depth=0 ルートも返す

**設計**: depth=0 のルート（steps が空）は「このターゲットは直接購入可能」を意味する。depth=0 ノードを routes 登録後も展開継続することで「購入可能だが合成ルートも知りたい」ケースに対応。

---

## L8: chematic の SA Score はアドミッシブルなヒューリスティックに使える

`chematic::chem::sa_score(mol)` → [1.0, 10.0]（1 = 簡単、10 = 困難）。

**実装**: `h = Σ(1.0 + 0.5 × (sa − 1) / 9)` — 最大値 1.5 per 分子 < 実ステップコスト最小値 1.0 → アドミッシブル性を維持。

---

## L9: PyO3 + maturin は `[lib]` + `[bin]` の共存が必要

**問題**: PyO3 は cdylib として lib をビルドするため、bin only プロジェクトでは使えない。

**対処**: `Cargo.toml` に `[lib]` と `[[bin]]` を両方宣言。`main.rs` は `use renkin::*` でライブラリを参照。

---

## L10: rayon は WASM でコンパイルできない — cfg で条件分岐

**対処**:
```toml
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
rayon = "1"
```

```rust
#[cfg(not(target_arch = "wasm32"))]
let expanded = rules.par_iter().flat_map(...).collect();
#[cfg(target_arch = "wasm32")]
let expanded = rules.iter().flat_map(...).collect();
```

---

## L11: wasm-bindgen 関数名は Rust の pub 関数名と衝突する

**問題**: `wasm.rs` で `#[wasm_bindgen] pub fn find_routes(...)` と `use crate::search::find_routes` が衝突して `E0255` エラー。

**対処**: `use crate::search::find_routes as rs_find_routes;` でエイリアスを付ける。

---

## L12: `maturin develop` は仮想環境が必要

**対処**: `python -m venv .venv && source .venv/bin/activate` を先に実行。プロジェクト直下の `.venv` は maturin が自動検出する。

---

## L13: PyO3 で Python 側の関数名を Rust 実装と切り離すには `#[pyo3(name = "...")]`

**対処**:
```rust
#[pyfunction]
#[pyo3(name = "find_routes", signature = (target, depth=5, ...))]
pub fn find_routes_py(...) -> PyResult<String> { ... }
```

---

## L14: USP TO-50k ベンチマークの失敗原因は「ルール不足」より「BB 不足」

**診断結果（2026-06-20）**:
- VF2 マッチングは正常（explicit-H SMILES も正しくマッチ）
- ルールは 93% の分子でフラグメントを生成している
- 失敗の主因: 生成フラグメントが 277件のBBに含まれない

**確認方法**: `trace_pipeline` テスト（`src/trace_test.rs`）で各ルールの発火とBB判定を可視化。

**改善**: BB 277件 → 480件超（医薬品合成頻出のアリールハライド、ボロン酸、ヘテロ環を追加）、ルール14件 → 20件（aryl C-Cl/I/F, Heck, Negishi を追加）で 2.6% → 5.0% に改善。

---

## L15: aryl C-Cl ルールが欠如していた（trace で発覚）

**問題**: `[CH3]c1[cH]nc([CH3])c([CH3])c1[Cl]`（クロロメチルピリジン）でルールが一切発火しない。aryl C-Cl 結合の切断ルールが存在しなかった。

**対処**: 4つの aryl C-halide ルールを追加:
- `aryl_chloride_retro`: `[c:1][Cl]>>[c:1]`
- `aryl_iodide_retro`: `[c:1][I]>>[c:1]`
- `aryl_fluoride_snAr_retro`: `[c:1][F]>>[c:1]`
- `aryl_chloride_to_bromide`: `[c:1][Cl]>>[c:1][Br]`

**教訓**: 「ルールが多ければ良い」ではなく、未発火ケースのトレースが最も効果的なデバッグ手法。

---

## L16: GitHub Pages に WASM プレイグラウンドをデプロイする際の注意点

**構成**:
1. `wasm-pack build --target web --out-dir site/pkg` で WASM を `site/pkg/` に出力
2. `mkdocs build -d site` でドキュメントを `site/` に出力（pkg/ は保持）
3. `docs/playground/index.html` を `site/playground/index.html` にコピー
4. プレイグラウンドの WASM ロードパスは `../pkg/renkin.js`（`site/pkg/` 相対）

**XSS 注意点**: `innerHTML` に外部入力（`e.message` 含む）を渡さず、`document.createTextNode()` + `appendChild()` で DOM 構築する。GitHub Actions のセキュリティフックが検出する。

**MkDocs の `nav:` で HTML ファイル直接参照**: `Playground: playground/index.html` と書けば MkDocs がそのまま `site/playground/index.html` にコピーする（変換しない）。

---

## L17: テンプレート頻度重み付け（Phase A）は同コーパス分割で効果大だが OOD は未検証

**知見（2026-06-22）**:
- rdchiral 抽出テンプレートの使用頻度（USPTO-50k 訓練セット）を `weight = ln(count+1)` でスコアリング
- ビームサーチの step_cost からこの重みに基づくボーナス（最大 0.2）を減算
- 対照実験（ボーナスなし）: 52%、Phase A あり: 72%（+20pp）

**なぜこんなに効くか**: beam=100 という制約下で、頻度重み付けがビーム刈り込みを「訓練分布に偏った探索」に変える。テスト分子が訓練と同一コーパスから来ているため、最も頻度の高いテンプレートが正解である確率が高い。これは強い in-domain バイアスであり、AiZynthFinder の NN template scoring も同原理だが、NN は分子構造情報も使うためより汎用的。

**注意点**: USPTO-50k 外（OOD: ChEMBL・天然物等）では Phase A の効果が激減する可能性がある。現時点で OOD 検証なし。

---

## L18: USPTO-50k 競合比較の落とし穴

**指標の混在（修正済み 2026-06-22）**:
- AiZynthFinder/Retro*/ASKCOS は「多段階経路探索成功率」（RENKIN と同種の指標）
- LocalRetro/GLG は「単ステップ top-1 予測精度」（別の指標）— RENKIN の 72.1% と直接比較してはならない

**条件の非対称性**:
- RENKIN の 72.1% は 2026 年社内計測、537 BB、500テンプレート
- AiZynthFinder の 45–53% は Genheden 2020 論文値、6M BB、50k テンプレート
- BB 数・テンプレート数・実装・時期すべてが異なり、matched-condition 実験は未実施

**正確な主張**: 「USPTO-50k 標準ベンチマーク（train/test 同コーパス分割）において 72.1% を達成（全件確認）。同指標の競合論文値（AiZynthFinder 45–53%、Retro* 44.3%、ASKCOS 41%）を数値上回るが、条件が異なるため優位性を断言するには matched-condition 実験が必要。OOD 汎化性は別途評価が必要。」

---

## L19: `simplify_smirks()` はテンプレートの適用範囲を広げる（false positive のリスクあり）

**実装**: `scripts/extract_templates.py` が rdchiral 抽出 SMIRKS から次数制約 `D1/D2/D3`・電荷制約 `+0`・`H0` を除去。

**メリット**: chematic が未対応の SMARTS 構文を回避し、より多くのテンプレートがロードされる。

**デメリット**: 元の rdchiral テンプレートより広い分子に適用されてしまい、化学的に正しくない分解（false positive）が生成される可能性がある。AiZynthFinder は RDKit でフル制約を使用しているため、この緩和は競合との非対称性の一因となりうる。

**要検証**: `simplify_smirks()` を無効化してフル制約で実行し、成功率への影響を定量化する（Phase 20.4）。

---

## L20: release の smoke test はパブリッシュ後にしか動かない — CI 事前ゲートが必要

**問題**: `release.yml` の `smoke-pypi` は PyPI publish 後に動く。smoke が壊れていても、タグを切った時点でパッケージは公開済み。v0.15.0〜v0.15.3 の全リリースが `renkin.version()` という存在しない関数呼び出しで失敗した。

**対処**: `ci.yml` に `python-smoke` ジョブを追加（`PyO3/maturin-action` でビルド → `__version__`・`find_routes` 等の API を検証）。master push のたびに動くので、タグを切る前に破壊を検出できる。

**教訓**: smoke test の「動かない日」は必ずくる。事後ではなく事前にゲートを置く。

---

## L21: GitHub branch protection の required check 名は workflow の `name:` フィールドと一致させる

**問題**: GitHub の branch protection に登録する status check 名は、workflow の job key (`test:`) ではなく `name: Test` フィールドと一致させる必要がある。不一致だと PR 上で "Expected — Waiting for status to be reported" になり、merge ボタンが永遠にグレーアウトする。

**対処**: `ci.yml` の各ジョブに `name:` を明示的に設定し、その文字列を `gh api` で branch protection に登録する。

---

## L22: PyPI 伝播は 60 秒で足りないことがある

**問題**: `release.yml` の `smoke-pypi` が `sleep 60; pip install renkin==${VERSION}` で PyPI の伝播を待っていたが、混雑時は 1〜5 分かかる。1 発失敗すると workflow が赤になる。

**対処**: `for i in 1 2 3 4 5; do sleep 60; pip install ... && break; done` でリトライループに変更。最大 5 分待つ。

---

## L23: バージョン不整合は CI で防ぐ — `docs/installation` と `README Citation` を自動検証

**問題**: Cargo.toml をバンプするとき docs/getting_started/installation.md の `renkin = "0.1"` や README の Citation `version = {0.15.2}` を更新し忘れる。手作業チェックは抜けやすい。

**対処**: `ci.yml` に `version-check` ジョブを追加。Cargo.toml からバージョンを抽出し、docs/ 内の `renkin = "0.x"` パターンと README Citation が一致しているかを grep で検証。不一致なら CI が赤くなる。
