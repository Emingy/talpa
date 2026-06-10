# assets

## wintun.dll (Windows builds only)

The Windows backend embeds the wintun driver DLL into the executable via
`include_bytes!("../../assets/wintun.dll")` (see `src/platform/windows.rs`), so
the built `.exe` is self-contained — no separate DLL needs to be shipped.

Before building **for Windows**, download the official wintun release from
<https://www.wintun.net> and copy the **x64** DLL here:

```
assets/wintun.dll   ←  wintun-<version>/bin/amd64/wintun.dll
```

This file is gitignored (it is a third-party binary). macOS/Linux builds do not
need it — `windows.rs` is compiled only on Windows targets.
