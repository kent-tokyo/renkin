# chematic への要望・バグ報告まとめ

> RENKIN が依存する化学情報処理ライブラリ `chematic` への GitHub Issue 投稿用メモ。
> 優先度順に記載。

## 投稿済み Issue サマリー

| Issue | 状態 | 内容 |
|---|---|---|
| [#13](https://github.com/kent-tokyo/chematic/issues/13) | ✅ CLOSED (0.4.12 修正) | BFS leakage in run_reactants |
| [#14](https://github.com/kent-tokyo/chematic/issues/14) | ✅ CLOSED (0.4.12 修正) | Non-deterministic canonical SMILES |
| [#15](https://github.com/kent-tokyo/chematic/issues/15) | ✅ CLOSED | run_reactants clean molecule guarantee |
| [#16](https://github.com/kent-tokyo/chematic/issues/16) | ✅ CLOSED | is_same_molecule() convenience function |
| [#18](https://github.com/kent-tokyo/chematic/issues/18) | 🔴 OPEN | run_reactants products contain bracket atoms ([O], [N]) |
| [#19](https://github.com/kent-tokyo/chematic/issues/19) | 🔴 OPEN | parse_smarts rejects atom-map notation (:N) |
| [#20](https://github.com/kent-tokyo/chematic/issues/20) | 🔴 OPEN | Feature: tetrahedral stereochemistry (@/@@) in run_reactants |

---

## 🔴 Bug #13: BFS leakage in run_reactants（最優先）

### 現象
`run_reactants(smirks, &[mol])` で SMIRKS に複数のアトムマップが含まれる場合、
**マップされていない原子が両方の産物テンプレートに重複コピー**される。

### 再現コード（Rust）

```rust
// アミド結合切断: CC(=O)Nc1ccccc1 → 酢酸 + アニリン のはずが
let mol = parse("CC(=O)Nc1ccccc1").unwrap();
let raw = run_reactants("[C:1](=[O:2])[N:3]>>[C:1](=[O:2])O.[N:3]", &[&mol]).unwrap();
// 期待: [["CC(=O)O", "Nc1ccccc1"]]
// 実際: [["C[C](O)=[O]", "c1cc(O)ccc1"], ["c1c(O)ccc(c1)N", "C"]]
//       → メタン(C)やフェノール(c1cc(O)ccc1)が余分に生成される
```

### 根本原因（推測）
SMIRKS の各産物テンプレートに属するアトムを決定する際、BFS が他のテンプレートの
マップ済みアトムの近傍にある非マップ原子まで展開してしまう。

### RENKIN での影響
- `amide_cleavage`、`suzuki_retro` など多くのルールに影響
- 現在はグラフ実装（`build_sub_molecule`）で回避しているが、SMIRKS ベースの新ルール追加が困難

### 望む修正
各産物テンプレートの BFS は、そのテンプレートに含まれる**マップ済みアトムに隣接する非マップ原子のみ**を取り込む。
他のテンプレートのマップ済みアトムを越えて展開しない。

---

## 🔴 Bug #14: Non-deterministic canonical SMILES

### 現象
同一分子の `canonical_smiles()` の出力が実行タイミングや分子の構築順序によって変わる可能性がある。

### RENKIN での影響
- BB ライブラリの VF2 マッチング（(atom_count, bond_count) キー）が不安定になる
- `is_building_block()` が同一分子を異なる canonical SMILES で照合し false negative を返す

### 現在の回避策
- `(atom_count, bond_count)` による事前フィルタリング + VF2 完全マッチングの組み合わせで緩和
- 完全な修正なしには、将来の大規模 BB DB（3M 件）での高速ハッシュ照合が実現できない

### 望む修正
`canonical_smiles()` がどの構築順序・実行タイミングでも同一分子に対して同一文字列を返す。

---

## 🟡 Feature Request: 立体化学 SMIRKS 対応（`@` / `@@`）

### 概要
SMIRKS 中の `@` / `@@` テトラヘドラルステレオを `run_reactants` で正しく処理してほしい。

### ユースケース
```
// 不斉 Diels-Alder など、立体化学を保持した retro SMIRKS
[C@@H:1]([OH:2])>>[C:1]=O
```

### 現状
chematic は CIP ステレオ記述をパースできるが、`run_reactants` での立体化学保持は未対応と思われる。

---

## 🟡 Feature Request: run_reactants に "atom-correct" モードの追加

### 概要
Bug #13 の根本修正が難しい場合の代替案として、
「マップ済みアトムのみを産物に含む厳密モード」オプションを `run_reactants` に追加してほしい。

```rust
// 提案 API
let raw = run_reactants_strict(smirks, &[mol], StrictMode::MappedOnly)?;
// → マップされていない原子は産物に含めない（SMIRKS のセマンティクス通り）
```

---

## 🟢 Feature Request: InChI / InChIKey 生成

### 概要
大規模 BB ライブラリ（3M 件 eMolecules）の重複除去・正規化に InChIKey を使いたい。

### ユースケース
```rust
use chematic::chem::inchi::inchi_key;
let key = inchi_key(&mol)?;  // "BSYNRYMUTXBXSQ-UHFFFAOYSA-N"
```

---

## 投稿先

- GitHub: https://github.com/reymond-group/chematic/issues（仮、実際のリポジトリURLを確認すること）
- 優先度: Bug #13 → Bug #14 → Feature Requests の順で投稿

## 監視コマンド

```bash
# chematic が更新されたか確認
cargo update chematic 2>&1 | grep -E "chematic|Updating"

# Bug #13 修正確認テスト
cargo test issue13 -- --nocapture

# Bug #14 修正確認テスト  
cargo test issue14 -- --nocapture
```
