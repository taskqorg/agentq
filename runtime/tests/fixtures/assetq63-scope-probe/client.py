import json,sys,pathlib
mode=sys.argv[1]; root=pathlib.Path(sys.argv[2])
def send(v): print(json.dumps(v),flush=True)
for line in sys.stdin:
 m=json.loads(line); root.joinpath('requests.jsonl').open('a').write(json.dumps(m)+'\n'); i=m.get('id'); method=m.get('method')
 if method=='initialize': send({'id':i,'result':{}})
 elif method=='thread/start':
  send({'id':i+99,'result':{'thread':{'id':'foreign'},'modelProvider':'foreign'}})
  send({'id':i,'result':{'thread':{'id':'owned'},'modelProvider':'openai','model':'fixture'}})
 elif method=='account/read':
  assert m['params']=={'refreshToken':False}
  if mode=='binding-request':send({'id':900,'method':'future/request','params':{'threadId':'owned'}})
  send({'id':i,'result':{'account':{'type':'chatgpt','planType':'plus'}}})
 elif method=='model/list':
  send({'id':i+99,'result':{'data':[{'model':'foreign'}]}})
  send({'id':i,'result':{'data':[{'model':'fixture'}],'nextCursor':None}})
 elif method=='turn/start':
  p={'threadId':'owned','turnId':'turn','item':{'id':'image','type':'imageGeneration','status':'completed','savedPath':'/never/open.png'}}
  row={'method':'item/completed','params':p}
  if mode=='foreign':p['threadId']='foreign'
  if mode=='unscoped':p.pop('turnId')
  if mode=='nested':p['turn']={'id':'foreign'}
  if mode=='alias':p['turn_id']='foreign'
  if mode=='header':row.update(type='thread.started',thread_id='foreign')
  if mode=='failed':p['item']['status']='failed'
  if mode=='unknown':p['item']['status']='future'
  send(row);send(row)
  if mode=='conflict':p['item']['unexposed']='conflicting';send(row)
  send({'method':'turn/completed','params':{'threadId':'owned','turn':{'id':'turn','status':'completed'}}})
  send({'id':i,'result':{'turn':{'id':'turn'}}})
 elif method=='turn/interrupt':send({'id':i,'result':{}})
