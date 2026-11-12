# Owned sample-map transfer

Status: experiment in progress. Production dependencies are unchanged.

`CrateData::try_field` returns an owned decoded `TimeSamples` value. The shared
opinion walk borrows its map for `SourceResolver`, which clones it into the
query source. Preserving that ownership through `SampleField` and
`OpinionResolver` would let the source take the decoded map directly. Borrowed
layer maps still need a clone. Accounting must use the retained capacities.
This changes data ownership, not sample selection or invalidation.

Before modifying that path, measure the current `7d91508d` prototype with a
64 KiB/eight-entry budget on the same three bird/six cow channels. Only the
small scale sources should be admitted (one bird/two cow). Larger rotation
and translation maps must be refused on every batch. Require exact answers,
bounded charges, zero charges after query drop, and at most 5% query overhead
versus direct reads in both first-read strata and both fresh-process orders.
Use 1,000 clocks plus four warmups per process and preserve all raw evidence.
This is the `mixed` protocol of `query-budget-probe/run.sh`.

After the ownership change, run the same protocol and the admitted/no-room
regressions. Retain original failures. Validate owned-buffer transfer and
borrowed-map preservation directly, and run core attribute/composition tests.
Kernel results do not establish whole-editor or rendered performance.
