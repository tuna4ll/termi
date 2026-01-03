# Termi

A lightweight, fast, and distraction-free terminal-based code editor written in Rust, designed for developers who want speed, simplicity, and full control directly from the command line.

![Rust](https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white)
![Platform](https://img.shields.io/badge/Platform-Windows-blue)
![License](https://img.shields.io/badge/License-MIT-green)

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+S` | Save |
| `Ctrl+C/X/V` | Copy/Cut/Paste |
| `Ctrl+Z/Y` | Undo/Redo |
| `Ctrl+F` | Search |
| `Ctrl+G` | Go to line |
| `Ctrl+A` | Select all |
| `Ctrl+T` | Terminal mode |
| `Ctrl+Q` | Quit |
| `F2` | Rename file |
| `Del` | Delete file |

## Installation (Recommended)
### Portable (No Installer)

1. Download the portable `.exe` from **[Releases](https://github.com/tuna4ll/termi/releases)**
2. Move it to a folder of your choice (example: `C:\Tools\termi`)
3. Add that folder to your **PATH** manually:

   * Open **Environment Variables**
   * Edit **Path**
   * Add the folder path
4. Restart terminal

---

### Build from Source (Advanced)

```bash
git clone https://github.com/tuna4ll/termi.git
cd termi
cargo build --release
```

Binary will be located at:

```
target/release/termi.exe
```

You may manually add this directory to your PATH if needed.

---

## Usage

```bash
termi
```

Use arrow keys to navigate the file tree, press `Enter` to open files.


## Support the Project

If you find **Termi** useful and enjoy working with it, you can support the project’s development.

Your support helps:
- New features
- Faster bug fixes
- Performance improvements
- Continued maintenance

### Crypto Donations

- **Bitcoin (BTC)**  
  `bc1qwgz582965w2augj5js65fj5qv8fkg2csxyhn5e`

- **Ethereum (ETH)**  
  `0xe05fbbB5497ec614a6aFc8C434E37f01C484A99A`

- **USDT**
  - TRC20: `TNkLpY4aSpdA5XnUGxUHt3aus9nAbcSjkZ`

Every contribution, no matter the size, is greatly appreciated.  
Thank you for supporting open-source ❤️

---

Made with ❤️ by [@tuna4l](https://github.com/tuna4ll)