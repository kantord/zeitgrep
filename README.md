# zeitgrep

Search **frecently‑edited** lines of code in your Git repository, ranked by how often **and** how recently a file has changed.

**zeitgrep** is a grep-like command that allows you to search files in a Git repository. The results are sorted by a frecency
score generate by [frecenfile](https://github.com/kantord/frecenfile). It uses [ripgrep](https://github.com/BurntSushi/ripgrep) as a search backend.  

## Usage example

<table><thead>
  <tr>
    <th>
      Unsorted grep results (
      <code>rg def | head</code>
      )
    </th>
  </tr></thead>
<tbody>
  <tr>
    <td><img src="https://github.com/user-attachments/assets/5b36a33a-e01f-4be4-9d41-932c3f23aa5e" /></td>
  </tr>
</tbody>
</table>

<table><thead>
  <tr>
    <th>
    zeitgrep results (
    <code>zg def | head</code>
    )
    </th>
  </tr></thead>
<tbody>
  <tr>
    <td><img src="https://github.com/user-attachments/assets/3fb5f950-73b2-4706-af3d-03f7c2d80527" /></td>
  </tr>
</tbody>
</table>



## ✨ Features

* **Ripgrep‑style regex search** over your Git repository
* Results **ranked by frecency** using the [`frecenfile`](https://crates.io/crates/frecenfile) library.
* Scalable to large repositories


## 📦 Installation

```bash
cargo install zeitgrep
```


## 🚀 Usage

```bash
zg {regular expression}
```

## 🧑‍🍳 Recipes

### Live grep in Neovim using `telescope.nvim`

To configure *[telescope.nvim](https://github.com/nvim-telescope/telescope.nvim)* to use `zeitgrep` for live grep, use the following:

```lua
require("telescope").setup {
  defaults = {
    vimgrep_arguments = {
      "zg",
      "--column",
      "--color=never",
    },
  },
}
```

### Find stale TODO

```bash
zg TODO --sort=asc
```

