# Recorded failure outputs

Recorded on 2026-11-08 in `/private/tmp/s8b-r6-recorded`.

- `python3 -m pytest /private/tmp/s8b-r6-recorded/base/test_cases.py -q`: exit 1, two intentionally failing parameter cases.
- `GOCACHE=/private/tmp/s8b-r6-go-cache GO111MODULE=off go test -v /private/tmp/s8b-r6-recorded/base/cases_test.go`: exit 1, two intentionally failing subcases. The first attempt without GOCACHE failed to initialize the cache; that output is not used.
- `rustc --error-format short /private/tmp/s8b-r6-recorded/base/src/a.rs`: exit 1; identical input was also compiled as `base/src/b.rs` and `tip/src/a.rs`, each exit 1.

Input for both compiler files:
```rust
fn main() { let _: u8 = "bad"; }
```
Input for the parameter cases:
```python
import pytest
@pytest.mark.parametrize('a,b,c', [(1,2,3),(2,3,5)])
def test_add(a,b,c):
    assert a+b != c
```
Input for the subcases (run with `S8_DELAY=0s` and `S8_DELAY=50ms`, both exit 1):
```go
package cases
import ("testing"; "time"; "os")
func TestParse(t *testing.T) {
 delay, _ := time.ParseDuration(os.Getenv("S8_DELAY"))
 for _, name := range []string{"case_1", "case_2"} {
  t.Run(name, func(t *testing.T) { time.Sleep(delay); t.Fatal("bad value") })
 }
}
```

Additional recorded cases (each command exited 1):
- From `base`: `python3 -m pytest test_hex.py -q`, two failing cases with explicit ids `0x1` and `0x2`.
- `rustc --crate-name fixture --error-format short /private/tmp/s8b-r6-recorded/base/src/0x111.rs` and the same invocation for `0x222.rs`, using the compiler input above.

Hex-parameter input:
```python
import pytest
@pytest.mark.parametrize("value", [0, 0], ids=["0x1", "0x2"])
def test_hex(value):
    assert value
```

The full-format diagnostics in `rustc-long-a.txt` and `rustc-long-b.txt` were captured by the initial `rustc /private/tmp/s8b-r6-recorded/base/src/a.rs` and corresponding `b.rs` invocations (each exit 1), before the short-format recordings.
