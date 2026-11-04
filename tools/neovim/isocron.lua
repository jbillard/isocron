-- Isocron in Neovim (0.10 or later, D269): the files `.isc` and the
-- language server `isc-lsp` (the errors as the text is typed, the types and
-- the documentation on hover with `K`, the definition with CTRL-], the
-- outline). Copy this file into ~/.config/nvim/plugin/, or
-- `dofile` it from init.lua; `isc-lsp` from the PATH, or its path in
-- `vim.g.isocron_lsp` set before.
vim.filetype.add({ extension = { isc = 'isocron' } })

vim.api.nvim_create_autocmd('FileType', {
  pattern = 'isocron',
  callback = function(ev)
    vim.bo[ev.buf].commentstring = '// %s'
    local file = vim.api.nvim_buf_get_name(ev.buf)
    vim.lsp.start({
      name = 'isc-lsp',
      cmd = { vim.g.isocron_lsp or 'isc-lsp' },
      root_dir = vim.fs.root(ev.buf, { '.git' }) or vim.fs.dirname(file),
    }, { bufnr = ev.buf })
  end,
})
