# Private stock policy

Apply a local vendor inventory and policy to every route leaf without sending
the route or stock data to a service:

```bash
renkin audit-route route.json \
  --private-stock private-vendors.csv \
  --stock-policy private-policy.json \
  --output json
```

The vendor table requires `smiles` and accepts `id`, `vendor`, `price`,
`lead_time_days`, `hazard`, and `available`. The policy is versioned JSON:

```json
{
  "schema_version": 1,
  "source_label": "internal-catalog",
  "source_revision": "2026-08-31",
  "allowed_vendors": ["Acme"],
  "blocked_vendors": ["Legacy"],
  "max_price": 100.0,
  "max_lead_time_days": 14,
  "blocked_hazards": ["flammable", "acute-toxic"],
  "require_available": true,
  "blocked_smiles": []
}
```

Each leaf is classified as `matched`, `rejected`, or `unknown`. Rejections
carry a stable reason such as `vendor_not_allowed`, `price_limit_exceeded`,
`lead_time_exceeded`, `hazard_blocked`, `not_available`, or
`prohibited_substance`. A missing
exact vendor record is `unknown`; salt-, stereo-, and tautomer-relaxed vendor
matches do not silently become exact stock identity.

`hazard` is an optional local catalog label. Matching is exact and
case-sensitive so each organization can define its own controlled vocabulary.

When multiple exact records satisfy the policy, the report selects one
deterministically: lowest known price, then shortest known lead time, then
vendor and catalog ID. This makes the selected offer reproducible without
uploading the vendor table.

For multi-route reports, `route_score` adds a deterministic planning signal:
routes are ranked first by fewer rejected leaves, then fewer unknown leaves,
fewer matched leaves with missing prices, lower known-price total, and shorter
maximum lead time. The rank is advisory and does not replace structural or
forward-validation status.

The output includes the policy source and a SHA-256 digest of the policy in
`audit_manifest.private_stock_policy_sha256`. The vendor table itself is not
uploaded or embedded in the report.
