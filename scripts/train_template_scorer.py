#!/usr/bin/env python3
"""
Train an MLP template relevance scorer and export it as ONNX.

Pipeline:
  1. Load templates from file -> {smirks: idx} index
  2. Load USPTO-50k training set (HuggingFace: bisectgroup/USPTO_50K)
  3. For each reaction: extract SMIRKS -> simplify -> look up index,
     compute ECFP4 (radius=2, 2048 bits) of product SMILES
  4. Train MLP: 2048 -> 1024 -> 512 -> N_templates (cross-entropy)
  5. Export as ONNX with input="input" [1,2048] and output="output" [1,N]

Usage:
    python3 scripts/train_template_scorer.py \
        --templates data/templates_extracted_5000.smi \
        --output data/template_scorer.onnx \
        [--epochs 50] [--batch-size 512] [--lr 1e-3]
"""

import argparse
import re
import sys
from pathlib import Path

import subprocess
import numpy as np
import torch
import torch.nn as nn
from datasets import load_dataset
from rdchiral.template_extractor import extract_from_reaction

# ── Template simplification (must match extract_templates.py exactly) ──────────

def _simplify_atom(atom_smarts: str) -> str:
    map_match = re.search(r':(\d+)', atom_smarts)
    atom_map = f":{map_match.group(1)}" if map_match else ""
    inner = atom_smarts[1:-1]
    inner_no_map = re.sub(r':\d+$', '', inner)
    parts = inner_no_map.split(';')
    base = parts[0]
    base = re.sub(r'D\d+', '', base)
    base = re.sub(r'H0', '', base)
    base = re.sub(r'\+0', '', base)
    base = base.strip(';').strip()
    result = f"[{base}{atom_map}]"
    return '' if result in ('[]', '[:]') else result


def simplify_smirks(smirks: str) -> str:
    return re.sub(r'\[[^\]]+\]', lambda m: _simplify_atom(m.group(0)), smirks)


# ── Fingerprint (via renkin-fp binary — must match Rust inference FP exactly) ──

_FP_BIN = "target/release/renkin-fp"

def ecfp4_batch(smiles_list: list) -> list:
    """Compute ECFP4 fingerprints via the renkin-fp binary (chematic FNV-1a).

    Returns a list of np.ndarray [2048] float32 or None for invalid SMILES.
    Bit space is identical to what the Rust scorer uses at inference time.
    """
    if not smiles_list:
        return []
    inp = "\n".join(smiles_list) + "\n"
    try:
        result = subprocess.run(
            [_FP_BIN], input=inp, capture_output=True, text=True, timeout=300
        )
    except FileNotFoundError:
        raise RuntimeError(
            f"renkin-fp binary not found at '{_FP_BIN}'. "
            "Build with: cargo build --release --features nn-scoring --bin renkin-fp"
        )
    if result.returncode != 0:
        raise RuntimeError(f"renkin-fp failed (exit {result.returncode}):\n{result.stderr}")
    fps = []
    for line in result.stdout.split("\n"):
        line = line.strip()
        if not line or line == "ERR":
            fps.append(None)
        else:
            arr = np.zeros(2048, dtype=np.float32)
            for bit_str in line.split():
                arr[int(bit_str)] = 1.0
            fps.append(arr)
    # Pad in case stdout has fewer lines (empty trailing newline)
    while len(fps) < len(smiles_list):
        fps.append(None)
    return fps


# ── MLP architecture (matches Rust scorer expectations) ───────────────────────

class TemplateScorer(nn.Module):
    def __init__(self, n_templates: int):
        super().__init__()
        self.net = nn.Sequential(
            nn.Linear(2048, 1024),
            nn.ReLU(),
            nn.Dropout(0.2),
            nn.Linear(1024, 512),
            nn.ReLU(),
            nn.Dropout(0.1),
            nn.Linear(512, n_templates),
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.net(x)


# ── Data loading ───────────────────────────────────────────────────────────────

def load_template_index(path: str) -> dict:
    idx = {}
    i = 0
    with open(path) as f:
        for line in f:
            if line.startswith('#') or not line.strip():
                continue
            smirks = line.split('\t')[0]
            idx[smirks] = i
            i += 1
    print(f"Loaded {len(idx)} templates from {path}", flush=True)
    return idx


def build_dataset(template_index: dict, verbose: bool = True):
    print("Loading USPTO-50k train split...", flush=True)
    ds = load_dataset("bisectgroup/USPTO_50K", split="train")
    print(f"  {len(ds)} reactions", flush=True)

    # Pass 1: extract (product_smiles, template_label) pairs
    pairs = []  # list of (smiles, label)
    skipped = {"no_template": 0, "not_in_index": 0}

    for i, row in enumerate(ds):
        if verbose and i % 5000 == 0:
            print(f"  pass1 {i}/{len(ds)} | pairs so far: {len(pairs)}", flush=True)

        try:
            result = extract_from_reaction({
                "reactants": row["reactants"],
                "products": row["product"],
                "_id": row["id"],
            })
            tmpl = result.get("reaction_smarts")
        except Exception:
            skipped["no_template"] += 1
            continue

        if not tmpl:
            skipped["no_template"] += 1
            continue

        simplified = simplify_smirks(tmpl)
        label = template_index.get(simplified)
        if label is None:
            skipped["not_in_index"] += 1
            continue

        pairs.append((row["product"], label))

    print(f"  pass1 done: {len(pairs)} candidate pairs", flush=True)

    # Pass 2: batch-compute chematic ECFP4 (identical bit-space to Rust inference)
    print("Computing chematic ECFP4 fingerprints via renkin-fp...", flush=True)
    smiles_list = [s for s, _ in pairs]
    fps = ecfp4_batch(smiles_list)

    X_list, y_list = [], []
    bad_fp = 0
    for fp, (_, label) in zip(fps, pairs):
        if fp is None:
            bad_fp += 1
        else:
            X_list.append(fp)
            y_list.append(label)

    print(
        f"\nDataset built: {len(X_list)} pairs  |  "
        f"skipped: no_template={skipped['no_template']}  "
        f"not_in_index={skipped['not_in_index']}  bad_fp={bad_fp}",
        flush=True,
    )
    return np.array(X_list, dtype=np.float32), np.array(y_list, dtype=np.int64)


# ── Training ───────────────────────────────────────────────────────────────────

def train_model(X, y, n_templates: int, epochs: int, batch_size: int, lr: float):
    device = torch.device("cpu")
    model = TemplateScorer(n_templates).to(device)
    optimizer = torch.optim.Adam(model.parameters(), lr=lr)
    criterion = nn.CrossEntropyLoss()

    X_t = torch.from_numpy(X)
    y_t = torch.from_numpy(y)
    n = len(X_t)

    for epoch in range(1, epochs + 1):
        model.train(True)
        perm = torch.randperm(n)
        X_t = X_t[perm]
        y_t = y_t[perm]

        total_loss = 0.0
        correct = 0
        batches = 0

        for start in range(0, n, batch_size):
            xb = X_t[start:start + batch_size].to(device)
            yb = y_t[start:start + batch_size].to(device)

            optimizer.zero_grad()
            logits = model(xb)
            loss = criterion(logits, yb)
            loss.backward()
            optimizer.step()

            total_loss += loss.item()
            correct += (logits.argmax(dim=1) == yb).sum().item()
            batches += 1

        acc = correct / n * 100
        avg_loss = total_loss / batches
        print(f"Epoch {epoch:3d}/{epochs}  loss={avg_loss:.4f}  acc={acc:.1f}%", flush=True)

    return model


# ── ONNX export ───────────────────────────────────────────────────────────────

def export_onnx(model, output_path: str) -> None:
    model.train(False)
    dummy = torch.zeros(1, 2048)
    torch.onnx.export(
        model,
        dummy,
        output_path,
        input_names=["input"],
        output_names=["output"],
        dynamic_axes={"input": {0: "batch"}, "output": {0: "batch"}},
        opset_version=11,
    )
    print(f"Exported ONNX model to {output_path}", flush=True)


# ── Main ──────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--templates", default="data/templates_extracted_5000.smi",
        help="Template file (SMIRKS<TAB>count lines)",
    )
    parser.add_argument(
        "--output", default="data/template_scorer.onnx",
        help="Path for the output ONNX model",
    )
    parser.add_argument("--epochs", type=int, default=50)
    parser.add_argument("--batch-size", type=int, default=512)
    parser.add_argument("--lr", type=float, default=1e-3)
    parser.add_argument(
        "--save-pt", default="",
        help="Also save trained weights as a .pt file (for re-export without retraining)",
    )
    args = parser.parse_args()

    template_index = load_template_index(args.templates)
    n_templates = len(template_index)

    X, y = build_dataset(template_index)

    if len(X) == 0:
        print("ERROR: no training pairs found.", file=sys.stderr)
        sys.exit(1)

    print(f"\nTraining MLP: 2048->1024->512->{n_templates} | {len(X)} samples", flush=True)
    model = train_model(X, y, n_templates, args.epochs, args.batch_size, args.lr)

    if args.save_pt:
        pt_path = args.save_pt
        torch.save({"state_dict": model.state_dict(), "n_templates": n_templates}, pt_path)
        print(f"Saved model weights to {pt_path}", flush=True)

    Path(args.output).parent.mkdir(parents=True, exist_ok=True)
    export_onnx(model, args.output)


if __name__ == "__main__":
    main()
