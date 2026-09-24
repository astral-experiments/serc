# Shared test and benchmark fixtures

These fixtures are shared by Serc's compatibility tests and benchmarks. The
`micro` and `marshal` fixtures are project-owned focused workloads. The
composite fixtures materialize source that was already covered by Serc's
compatibility and execution tests.

`large/dataset.py` is copied byte-for-byte from
`crates/ruff_benchmark/resources/large/dataset.py` at the Serc workspace's
pinned Ruff commit `e4a063a136ae0e3d6801bd0dad47d09a0f419530`. Its SHA-256
digest is `7cb3740caf21534f7418df1666686e2eade44a7405d825355289de58aac3acf2`.

Ruff attributes that resource to
[`tests/test_dataset.py`](https://github.com/DHI/mikeio/blob/b7d26418f4db2909b0aa965253dbe83194d7bb5b/tests/test_dataset.py)
from MikeIO commit `b7d26418f4db2909b0aa965253dbe83194d7bb5b`. It remains under
its original license.
