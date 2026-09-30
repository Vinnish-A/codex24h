#!/usr/bin/env python3
import os,sys,json,uuid,tempfile,shutil,subprocess,time
from pathlib import Path
import e2e
import argparse
source_home=Path(os.environ.get('CODEX_HOME',str(Path.home()/'.codex')))
parser=argparse.ArgumentParser(description='Read-only native history test using a temporary copy of a completed session; no model requests.')
parser.add_argument('rollout',type=Path)
parser.add_argument('--tmux-client',action='store_true',help='send keys through a real tmux client PTY, including its Escape timeout')
parser.add_argument('--size',default='186x46',help='terminal COLSxROWS')
parser.add_argument('--draft',action='store_true',help='also verify that full history preserves the native draft')
args=parser.parse_args()
cols,rows=map(int,args.size.split("x"))
source=args.rollout
source_id=next(json.loads(line)['payload']['id'] for line in source.open() if json.loads(line).get('type')=='session_meta')
with tempfile.TemporaryDirectory(prefix='codex24h-render-check-') as tmp:
 base=Path(tmp);home=base/'home';home.mkdir(mode=0o700)
 for name in ('auth.json','config.toml'):
  if (source_home/name).exists():(home/name).symlink_to(source_home/name)
 session=str(uuid.uuid4());folder=home/'sessions'/'2026'/'09'/'30';folder.mkdir(parents=True)
 target=folder/source.name.replace(source_id,session)
 records=[]
 for raw in source.read_text().replace(source_id,session).splitlines():
  r=json.loads(raw)
  if 'cwd' in r.get('payload',{}):r['payload']['cwd']=str(e2e.ROOT)
  records.append(json.dumps(r,ensure_ascii=False))
 target.write_text('\n'.join(records)+'\n');target.chmod(0o600)
 socket=str(base/'tmux.sock')
 def tmux(*args):return subprocess.check_output(['tmux','-S',socket,*args],text=True).strip()
 import shlex
 cmd=['env','CODEX_HOME='+str(home),'CODEX24H_MAIL=0',str(e2e.BIN),'--no-daemon','-c','notify=[]','resume',session]
 pane=tmux('-f','/dev/null','new-session','-d','-s','probe','-x',str(cols),'-y',str(rows),'-c',str(e2e.ROOT),'-P','-F','#{pane_id}',shlex.join(cmd))
 client=None
 if args.tmux_client:
  binary=e2e.BIN
  try:
   e2e.BIN=Path(shutil.which('tmux'))
   client=e2e.Terminal(os.environ|{'TERM':'xterm-256color'},['-S',socket,'attach-session','-t','probe'])
   client.resize(rows+1,cols)
  finally:e2e.BIN=binary
 def send(b):
  if client:client.send(b)
  else:tmux('send-keys','-t',pane,'-H',*[f'{v:02x}' for v in b])
 try:
  chosen=False
  for _ in range(150):
   if client:client.drain(.01)
   screen=tmux('capture-pane','-p','-t',pane)
   if 'Working directory' in screen and not chosen:
    send(b'\x1b[B\r');chosen=True
   if 'Ask Codex' in screen and _>20:break
   time.sleep(.2)
  print('startup ready:', 'Ask Codex' in screen,flush=True)
  import importlib.machinery
  h=importlib.machinery.SourceFileLoader('h',str(e2e.ROOT/'scripts/codex24h-requests')).load_module()
  req=[r['text'] for r in h.read_session(source)['requests']]
  def wait(pred, name, timeout=15):
   until=time.monotonic()+timeout
   while time.monotonic()<until:
    if client:client.drain(.01)
    view=tmux('capture-pane','-p','-t',pane)
    if pred(view):return view
    time.sleep(.1)
   if os.environ.get('CODEX24H_TEST_CAPTURE'):
    Path(os.environ['CODEX24H_TEST_CAPTURE']).write_text(view)
   raise RuntimeError(name+' timed out; footer: '+repr(view.splitlines()[-1]))
  draft='draft-check-'+session[:8]
  if args.draft:
   send(b'\x1b[200~'+draft.encode()+b'\x1b[201~')
   wait(lambda v:draft in v,'native draft accepted')
  for index in range(len(req)):
   send(b'\x1dr');wait(lambda v:f'Requests ({len(req)})' in v,'list')
   send(b'\x1b[H'+b'\x1b[B'*index+b'\r')
   lines=req[index].splitlines()
   query=next((s.strip() for s in lines if len(s.strip())>=12),next((s.strip() for s in lines if s.strip()),''))[:160]
   def body(view):
    # Find occupies a variable number of footer rows. Never count its query as
    # evidence that a request appeared in the actual conversation.
    lines=view.splitlines()
    stop=next((i for i,line in enumerate(lines) if line.lstrip().startswith('Find:')),len(lines)-1)
    if 'NATIVE HISTORY' in view:return lines[1:stop]
    stop=next((i for i,line in enumerate(lines) if '-- live input' in line),stop)
    return lines[:stop]
   def found(view):
    text=''.join(''.join(body(view)).split())
    return ''.join(query.split()) in text and ('NATIVE HISTORY' in view or 'native terminal history' in view) and ' · searching' not in view

   view=wait(found,'request '+str(index+1),25)
   print('PASS request',index+1,'native full history' if 'NATIVE HISTORY' in view else 'terminal anchor',flush=True)
   native='NATIVE HISTORY' in view
   original=body(view)
   # First wheel must move within this context, not restore the pre-search page.
   send(b'\x1b[<65;30;10M')
   moved=wait(lambda v:body(v)!=original,'wheel down')
   shared=set(original)&set(body(moved));shared.discard('')
   assert len(shared)>=max(1,len(set(original)-{''})//3),'wheel lost the located context'
   send(b'\x1b[<64;30;10M')
   wait(lambda v:body(v)==original,'wheel round trip')
   send(b'\x1b[6~')
   wait(lambda v:body(v)!=original,'page down')
   send(b'\x1b[5~')
   wait(lambda v:body(v)==original,'page round trip')
   send(b'\x03')
   wait(lambda v:'T R A N S C R I P T' not in v and 'HISTORY (frozen)' not in v and 'NATIVE HISTORY' not in v,'one Ctrl+C closes')
   print('PASS request',index+1,'wheel/page round trips and one Ctrl+C',flush=True)

   if args.draft:wait(lambda v:draft in v and 'T R A N S C R I P T' not in v,'native draft preserved')
  # Exercise the fallback UI itself, beyond finding the requested text.
  first_line=next(s.strip() for s in req[0].splitlines() if len(s.strip())>=12)
  query=first_line[:160]
  def first():
   send(b'\x1dr');wait(lambda v:f'Requests ({len(req)})' in v,'reopen list')
   send(b'\x1b[H\r')
   return wait(found,'reopen first request',25)
  view=first()
  if 'NATIVE HISTORY' in view:
   # In reading mode these keys select requests, not repeated text matches.
   original_query=query
   next_query=next((s.strip() for s in req[1].splitlines() if len(s.strip())>=12),req[1].strip())[:160]
   send(b'\r');query=next_query
   wait(lambda v:'NATIVE HISTORY 2/' in v and found(v),'Enter selects next request',25)
   send(b'\x10');query=original_query
   wait(lambda v:'NATIVE HISTORY 1/' in v and found(v),'Ctrl+P selects previous request',25)
   send(b'\x10')
   wait(lambda v:'First request' in v,'first request boundary')
   # Resize while reading, then return to the original geometry.
   tmux('resize-window','-t','probe','-x','80','-y','24')
   wait(lambda v:78<=len(v.splitlines()[0])<=80 and found(v),'narrow native history')
   tmux('resize-window','-t','probe','-x',str(cols),'-y',str(rows))
   wait(lambda v:cols-2<=len(v.splitlines()[0])<=cols and found(v),'restore native history width')
   send(b'\x1b[F')
   wait(lambda v:'Find:' not in v and 'q close' in v,'End to native latest')
   # Slash and the full paste deliberately arrive together, before Find draws.
   send(b'/\x1b[200~no-such-query-'+session.encode()+b'\x1b[201~')
   wait(lambda v:'No matches' in v,'no-match feedback')
   send(b'\x15\x1b[200~'+query.encode()+b'\x1b[201~')
   wait(lambda v:''.join(query.split()) in ''.join(''.join(body(v)).split()) and 'enter next' in v,'refined search body')
   send(b'\x1b[<65;30;10M')
   wait(lambda v:'NATIVE FIND' in v,'keyword search survives scrolling')
   send(b'\x10')
   wait(lambda v:'No more matches' in v and 'NATIVE FIND' in v,'keyword previous match boundary')
   # A keyword with multiple hits must move between matches, including after
   # scrolling. Check conversation rows, not the search input or status label.
   send(b'/\x1b[200~git\x1b[201~')
   multi=wait(lambda v:'enter next' in v and 'NATIVE FIND' in v and 'No more matches' not in v and 'git' in ''.join(body(v)).lower(),'keyword with multiple matches')
   original_match=body(multi)
   send(b'\x10')
   wait(lambda v:body(v)!=original_match and 'NATIVE FIND' in v and ' · searching' not in v,'previous keyword match')
   send(b'\r')
   wait(lambda v:body(v)==original_match and 'NATIVE FIND' in v,'next keyword match returns')
   # Reopening the request picker waits for the native transcript to close.
   send(b'\x1dr')
   wait(lambda v:f'Requests ({len(req)})' in v and 'T R A N S C R I P T' not in v,'picker from native history')
   send(b'\x03')
   if args.draft:wait(lambda v:draft in v,'draft after picker cancel')
   for key,label in [(b'\x1b','Escape'),(b'\x1b\x1b','double Escape'),(b'q','q')]:
    first();started=time.monotonic();send(key)
    wait(lambda v:'T R A N S C R I P T' not in v and 'NATIVE HISTORY' not in v,'close with '+label,2)
    print('PASS',label,'close latency',round(time.monotonic()-started,3),'seconds',flush=True)
    if args.draft:wait(lambda v:draft in v,'draft after '+label)
   # Cancel immediately after requesting another full-history search. An
   # immediate duplicate Escape stays local until the close is acknowledged.
   first();started=time.monotonic();send(b'\r\x1b[27u\x1b[27u')
   wait(lambda v:'T R A N S C R I P T' not in v and 'NATIVE HISTORY' not in v,'cancel during request navigation',2)
   if args.draft:wait(lambda v:draft in v,'draft after cancellation')
   print('PASS cancel during navigation latency',round(time.monotonic()-started,3),'seconds',flush=True)
   print('PASS request next/previous, resize, no-match/refined search, picker reopening, Escape/q and drafts',flush=True)
  assert [r['text'] for r in h.read_session(target)['requests']] == req, 'navigation submitted a new request'
  print('PASS all',len(req),'requests, no model task submitted',flush=True)
 finally:
  if client:client.close()
  subprocess.run(['tmux','-S',socket,'kill-server'],capture_output=True)
