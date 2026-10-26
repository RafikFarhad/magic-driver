# Test fixtures

Recorded raw report streams used by scenario tests (ARCHITECTURE 7.2). Every fixture can be replayed without hardware.

## Format

Fixtures use the recording format from ARCHITECTURE 5.2, implemented in `md-core::recording`:

```text
magic-driver-recording format=1 profile=magic-mouse-2-bt magic-driver=0.1.0 kernel=7.1.13-2-MANJARO start=1767225600
0 1200000000000000000000000000
11111 1200000000000000000000000000...
```

- Line 1 is the header: format version, profile ID, magic-driver version, kernel release and the wall-clock start time in Unix seconds. Field values never contain whitespace or `=`.
- Every other line is one report: microseconds since the start of the recording, one space, the report bytes as contiguous lowercase hex.
- Timestamps never decrease. Trailing whitespace, CRLF line endings and blank lines are tolerated; anything else malformed is rejected with its line number.

## Naming

```text
tests/fixtures/<profile id>/<scenario>.rec
```

- `<profile id>` is the `id` of the device profile, for example `magic-mouse-2-bt`.
- `<scenario>` is lowercase kebab-case and says what the hand does, for example `idle-resting-palm`, `move-fingers-resting`, `slow-scroll`, `flick`, `horizontal-scroll`, `click-left`, `click-right`, `click-middle`, `click-while-touching`, `two-finger-swipe`, `two-finger-double-tap`, `connect-first-touch`.
- Several takes of the same scenario get a numeric suffix: `slow-scroll-2.rec`.
- Expected engine outputs are not stored here. They are insta snapshots next to the test that replays the fixture (`crates/*/tests/snapshots/`).

## How fixtures are produced

Real fixtures are recorded by the owner only, never by an implementer (ARCHITECTURE 7.5):

1. The owner runs `magic-driver record tests/fixtures/magic-mouse-2-bt/<scenario>.rec` (available from E2-T07). It reads the raw device without grabbing it and refuses to overwrite an existing file.
2. The owner performs the scenario with the real mouse and stops the recording with Ctrl+C.
3. The owner commits the file. The list of fixtures wanted for M0 is in E2-T10.

Before any real recording exists, tests build reports with `md_core::testkit::ReportBuilder` (enable the `test-support` feature of `md-core` as a dev-dependency) and write them with `md_core::recording::RecordingWriter`. `crates/md-core/tests/scenario_example.rs` shows the pattern.

Recordings contain only touch, button and motion data from the mouse plus the header fields above. They hold no device serial number or Bluetooth address.
