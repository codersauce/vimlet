#!/usr/bin/env python3
"""Compare Vimlet with pinned Neovim using real queued input and bundled maps.

No user configuration, ShaDa, swap, or logs touch the checkout. Missing Neovim,
wrong versions, driver failures, and mismatches fail instead of skipping.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "NVIM v0.12.1"

def observe(case, executable, scratch):
    config = json.dumps(case, ensure_ascii=False)
    script = """
local case = vim.json.decode(%s)
vim.o.swapfile = false
vim.o.shadafile = 'NONE'
vim.o.modeline = false
vim.o.selection = 'inclusive'
vim.o.virtualedit = ''
vim.o.startofline = true
vim.o.whichwrap = 'b,s'
vim.o.joinspaces = false
vim.o.clipboard = ''
local input_path = vim.fn.getcwd() .. '/input.txt'
vim.fn.writefile(vim.split(case.text, '\\n', {plain=true}), input_path)
vim.cmd('edit! ' .. vim.fn.fnameescape(input_path))
local prefix = string.sub(case.text, 1, case.cursor or 0)
local _, line = string.gsub(prefix, '\\n', '')
local last = string.match(prefix, '.*\\n()') or 1
vim.api.nvim_win_set_cursor(0, {line+1, #prefix-last+1})
-- Flush setup history: user keys must be the first undoable change.
vim.bo.undolevels = -1
vim.bo.undolevels = 1000
local keys = vim.api.nvim_replace_termcodes(case.keys, true, false, true)
vim.api.nvim_feedkeys(keys, 't', false)
vim.defer_fn(function()
  local lines = vim.api.nvim_buf_get_lines(0, 0, -1, false)
  local cursor = vim.api.nvim_win_get_cursor(0)
  local offset = cursor[2]
  for i=1,cursor[1]-1 do offset = offset + #lines[i] + 1 end
  local reg = vim.fn.getreginfo('"')
  local content = table.concat(reg.regcontents or {}, '\\n')
  local typ = reg.regtype or ''
  if typ == 'V' then content = content .. '\\n' end
  local value = {text=table.concat(lines, '\\n'),cursor=offset,mode=vim.fn.mode(1),register=content,register_type=typ}
  io.stdout:write('VIMLET_RESULT:' .. vim.json.encode(value) .. '\\n')
  io.stdout:flush()
  vim.cmd('qa!')
end, 15)
""" % json.dumps(config, ensure_ascii=False)
    script_path = scratch / "observe.lua"
    script_path.write_text(script)
    env = dict(os.environ, NVIM_LOG_FILE=str(scratch / "nvim.log"), XDG_STATE_HOME=str(scratch / "state"))
    result = subprocess.run([executable, "--headless", "-u", "NONE", "--noplugin", "-i", "NONE", "-n", "-c", f"luafile {script_path}"], cwd=scratch, env=env, capture_output=True, text=True, timeout=10)
    combined = result.stdout + result.stderr
    if result.returncode:
        raise RuntimeError(combined)
    for line in combined.splitlines():
        if "VIMLET_RESULT:" in line:
            return json.loads(line.split("VIMLET_RESULT:", 1)[1])
    raise RuntimeError(f"No reference observation: {combined}")

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--nvim",default=os.environ.get("NVIM", "nvim"))
    parser.add_argument("--cases",type=Path,default=ROOT/"tests/conformance.json")
    parser.add_argument("--record",action="store_true",help="Print reference observations only")
    args=parser.parse_args()
    version=subprocess.check_output([args.nvim,"--version"],text=True).splitlines()[0]
    if version != VERSION:
        raise SystemExit(f"Expected {VERSION}, got {version}")
    cases=json.loads(args.cases.read_text())
    if not args.record:
        subprocess.run(["cargo","build","--locked","--example","trace"],cwd=ROOT,check=True)
        result=subprocess.run([str(ROOT/"target/debug/examples/trace")],input="".join(json.dumps(c)+"\n" for c in cases),capture_output=True,text=True,check=True)
        actual=[json.loads(line) for line in result.stdout.splitlines()]
        if len(actual)!=len(cases):raise RuntimeError("Driver observation count differs")
    failures=[]
    with tempfile.TemporaryDirectory(prefix="vimlet-nvim-") as directory:
        scratch=Path(directory)
        for index,case in enumerate(cases):
            expected=observe(case,args.nvim,scratch)
            if args.record:print(json.dumps(dict(case=case,expected=expected),ensure_ascii=False));continue
            fields=case.get("check",["text","cursor","mode","register","register_type"])
            delta={field:{"nvim":expected[field],"vimlet":actual[index][field]} for field in fields if expected[field]!=actual[index][field]}
            if delta:failures.append((case.get("name",case["keys"]),delta))
    for name,delta in failures:print(f"FAIL {name}: {json.dumps(delta,ensure_ascii=False)}")
    if failures:raise SystemExit(f"{len(failures)}/{len(cases)} conformance cases failed")
    if not args.record:print(f"{len(cases)}/{len(cases)} Neovim {VERSION.removeprefix('NVIM v')} cases passed")
if __name__=="__main__":main()
