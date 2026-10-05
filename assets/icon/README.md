# Application icon

Prepared from the project maintainer's `Neon RGB Mechanical Keycap Icon.png`.
The original file is left untouched and untracked at the checkout root.
Source SHA-256: `364d8c95e19578e17c896a418b42edf0bca80acaab4590228316a8bec839485e`.

- `underglow.png`: edited 1024px PNG with a transparent exterior and
  antialiased tile boundary. The keycap, bevel, and RGB lighting are retained.
- `underglow-256.png`: embedded GUI/Dock icon for unbundled development runs.
- `Underglow.icns`: macOS bundle icon, with standard and Retina representations
  from 16 through 1024 pixels.

Normal builds use these checked-in files; no image tools are runtime dependencies.
To regenerate on macOS (Apple command-line tools required):

```sh
swift scripts/prepare-icon.swift 'Neon RGB Mechanical Keycap Icon.png' assets/icon
```

The outline in that script is traced for this specific artwork. Different
artwork needs a new outline; this is not a generic background-removal tool.
The script does not overwrite its input or launch/install the application.
