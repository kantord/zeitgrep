# zeitgrep

Search **frecently‑edited** lines of code in your Git repository, ranked by how often **and** how recently a file has changed.

**zeitgrep** is a grep-like command that allows you to search files in a Git repository. The results are sorted by a frecency
score generate by [frecenfile](https://github.com/kantord/frecenfile). It uses [ripgrep](https://github.com/BurntSushi/ripgrep) as a search backend.

## ✨ Features

* **Ripgrep‑style regex search** over your Git repository
* Results **ranked by frecency** using the [`frecenfile`](https://crates.io/crates/frecenfile) library.
* Scalable to large repositories


## 📦 Installation

```bash
cargo install frecentgrep
```


## 🚀 Usage

```bash
frecentgrep {regular expression}
```

