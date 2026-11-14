-- Run with Neovim 0.12.1 from the repository root or pass the fixture path.
assert(vim.version().major == 0 and vim.version().minor == 12 and vim.version().patch == 1)
local path = arg[1] or 'tests/command_line.json'
local cases = vim.json.decode(table.concat(vim.fn.readfile(path), '\n'))
local observed
local function observe()
  observed = { text = vim.fn.getcmdline(), caret = vim.fn.getcmdpos() - 1, kind = vim.fn.getcmdtype() }
  return ''
end
vim.keymap.set('c', '<F12>', observe, { expr = true })

local prefixes = { Ex = ':', SearchForward = '/', SearchBackward = '?', Field = '@' }
local keys = {
  Left = '<Left>', Backspace = '<BS>', Delete = '<Del>', Enter = '<CR>',
  Escape = '<Esc>', ['Ctrl-u'] = '<C-U>', ['Ctrl-w'] = '<C-W>',
}
local function feed(s, flags)
  vim.api.nvim_feedkeys(vim.api.nvim_replace_termcodes(s, true, false, true), flags, false)
end
for _, case in ipairs(cases) do
  observed = nil
  local input, paste = '', nil
  for _, k in ipairs(case.keys) do
    if k:sub(1, 6) == 'Paste:' then
      paste = k:sub(7)
    else
      input = input .. (keys[k] or k)
    end
  end
  if case.kind == 'Field' then
    feed(input .. (case.text and '<F12><CR>' or ''), 't')
    local accepted = vim.fn.input({ prompt = 'Field: ', default = case.initial or '', cancelreturn = 'cancelled' })
    if not case.text then
      assert(accepted == case.outcome, vim.inspect(case))
    end
  elseif paste then
    vim.keymap.set('c', '<F11>', function()
      vim.schedule(function()
        vim.api.nvim_paste(paste, true, -1)
        feed('<F12><C-C>', 't')
      end)
      return ''
    end, { expr = true })
    -- input() blocks on command-line input, so scheduled paste runs in that mode.
    feed(input .. '<F11>', 't')
    vim.fn.input('Paste: ')
  elseif case.text then
    feed(prefixes[case.kind] .. input .. '<F12><C-C>', 'xt')
  else
    feed(prefixes[case.kind] .. input, 'xt')
    assert(vim.fn.getcmdtype() == '', vim.inspect(case))
  end
  if case.text then
    assert(vim.deep_equal(observed, { text = case.text, caret = case.caret, kind = paste and '@' or prefixes[case.kind] }),
      vim.inspect({ case = case, observed = observed }))
  end
end
print(('%d command-line cases match Neovim'):format(#cases))
