# Budgeted animation queries

Status: schema integration in an isolated upstream candidate. Production
dependencies and runtime remain unchanged.

`AnimQuery::with_budget` retains four attribute-query handles for translations,
rotations, scales and blend-shape weights. Source admission uses the shared
`QueryCacheBudget`; clones start with empty memos under the same budget.
`AnimQuery::new` keeps direct reads. The optional query handles are boxed, so
the direct mode does not embed four unused query objects.

Both modes share typed array conversion and short-circuit reads when a required
component is missing or invalid. Joint and blend-shape orderings remain
construction-time snapshots. A persistent application owner must rebuild
queries on ordering edits, bound handles/keys separately from source storage,
and release queries on close/replacement. This API alone is not that owner.

## Declared schema-API gate

Compare direct and budgeted `AnimQuery::joint_local_transforms` on the one
bird/two cow animations identified by the saved channel trace. Use 1,000 unique
clocks and four warmups per process, balanced first-read order and reversed
fresh-process order. Require exact optional-result presence, joint counts and
all matrix bits; match fingerprints across fresh processes and admission
policies. Preserve raw samples, source/asset identities, host load and failures.

- An 8 MiB/eight-entry budget must retain the traced three/six component
  sources, remain within both limits, release all charges on drop, and lower
  each first-read stratum's median by at least 25%.
- Zero-byte, one-byte and zero-entry budgets must retain no sources and have
  no more than 5% median overhead in any first-read/order/asset stratum.
- A 64 KiB/eight-entry budget must retain only the one/two small scale sources
  and meet the same 5% overhead limit while larger maps are refused.

This includes typed conversion and local-matrix construction. It excludes
skeleton-order remapping, skinning, mesh preparation, scene publication and
rendering. Passing it cannot establish a whole-editor gain or native 60 Hz.
The changed direct path also needs a comparison with the original executable
before production adoption; within-candidate policy comparisons do not prove
that the default constructor is regression-free.
