"""从准确客户端公开构造 / 库存 / NBT API 导出容器容量和默认数据，无世界、无窗口、无反射。"""
import argparse,base64,hashlib,json,pathlib,re,subprocess,tempfile
from client_reference import run_source_reference

def main():
 p=argparse.ArgumentParser(description=__doc__)
 for name in ['client-jar','version-json','libraries','output']:p.add_argument('--'+name,type=pathlib.Path,required=True)
 args=p.parse_args();version=json.loads(args.version_json.read_text(encoding='utf-8'))['id']
 common='import java.io.*;import java.nio.file.*;import java.util.*;import com.google.gson.*;\npublic class ContainerReference {public static void main(String[] args)throws Exception {\n var stderr=System.err;try{BOOTSTRAP}finally{System.setErr(stderr);}\n LOOKUP\n var result=new TreeMap<String,JsonObject>();\n LOOP {\n  var be=CREATE;\n  if(be==null)continue;\n  int size=SIZE;\n  var tag=SAVE;\n  String id=ID;\n  if(result.containsKey(id))continue;\n  var output=new ByteArrayOutputStream();var data=new DataOutputStream(output);WRITE;\n  var entry=new JsonObject();entry.addProperty("size",size);entry.addProperty("block",BLOCK);entry.addProperty("named",NAMED);entry.addProperty("nbt",Base64.getEncoder().encodeToString(output.toByteArray()));result.put(id,entry);\n }\n Files.writeString(Path.of(args[0]),new Gson().toJson(result));System.out.println("容器参照："+result.size());\n}}'
 if version=='1.21.1':
  api=dict(BLOCK='block.o().toString()',NAMED='be instanceof dqb',BOOTSTRAP='ab.a();akt.a();',LOOKUP='jo.a lookup=ka.a(lt.aA);',LOOP='for(var block:lt.e)if(block instanceof diq factory)',CREATE='factory.a(jd.c,block.o())',SIZE='be instanceof bqk container?container.b():be instanceof dqn campfire?campfire.b().size():0',SAVE='be.b(lookup)',ID='tag.l("id")',WRITE='uo.a(tag,(DataOutput)data)')
 elif version=='1.20.1':
  api=dict(BLOCK='block.n().toString()',NAMED='be instanceof czh',BOOTSTRAP='aa.a();acs.a();',LOOKUP='',LOOP='for(var block:jb.f)if(block instanceof csb factory)',CREATE='factory.a(gu.b,block.n())',SIZE='be instanceof bdq container?container.b():be instanceof czt campfire?campfire.c().size():0',SAVE='be.m()',ID='tag.l("id")',WRITE='rb.a(tag,(DataOutput)data)')
 else:
  code=subprocess.check_output(['javap','-classpath',str(args.client_jar),'-p','-c','avj'],text=True,encoding='utf-8')
  classes=re.findall(r'// String ([a-z_]+)\s+\d+: ldc(?:_w)?\s+#\d+\s+// class ([\w$]+)\s+\d+: invokestatic\s+#\d+\s+// Method a:\(Ljava/lang/String;Ljava/lang/Class;\)V',code)
  if len(classes)!=25:raise ValueError('旧方块实体注册表数量不符')
  registry=json.loads(pathlib.Path(__file__).resolve().parents[1].joinpath('crates/mcedit2-world/assets/block-entity-types-1343.json').read_text(encoding='utf-8'))['types']
  block_ids={key:int(next(v.split(':')[1]for v in values if v.startswith('numeric:')))for key,values in registry.items()}
  api=dict(BLOCK='"numeric:"+blocks.get(id)+"[data="+aow.c(blocks.get(id)).e(aow.c(blocks.get(id)).t())+"]"',NAMED='be instanceof avx',BOOTSTRAP='ni.c();',LOOKUP='var blocks=Map.ofEntries('+','.join('Map.entry('+json.dumps(id)+','+str(n)+')'for id,n in block_ids.items())+');',LOOP='for(avj be:new avj[]{'+','.join('new '+c.replace('$','.')+'()' for id,c in classes if id in {'furnace','chest','dispenser','dropper','brewing_stand','hopper','shulker_box','jukebox','sign','command_block'})+'})',CREATE='be',SIZE='be instanceof tv container?container.w_():0',SAVE='be.b(new fy())',ID='tag.l("id")',WRITE='gi.a(tag,(DataOutput)data)')
  common=common.replace('  var be=CREATE;','')
 for key in sorted(api,key=len,reverse=True):common=common.replace(key,api[key])
 base=pathlib.Path(__file__).resolve().parents[1]/'target/item-reference'
 with tempfile.TemporaryDirectory(prefix='containers-',dir=base)as temporary:proof=run_source_reference(args,'ContainerReference',common,pathlib.Path(temporary),version)
 data=json.loads(args.output.read_text(encoding='utf-8'))
 for entry in data.values():
  entry['nbt']=list(base64.b64decode(entry['nbt']))
  if entry['block'].startswith('Block{'):
   match=re.fullmatch(r'Block\{([^}]+)\}(.*)',entry['block'])
   if not match:raise ValueError('原版状态字符串格式不符')
   entry['block']=match[1]+match[2]
 args.output.write_text(json.dumps(data,separators=(',',':'))+'\n',encoding='utf-8',newline='\n')
 proof['output_sha256']=hashlib.sha256(args.output.read_bytes()).hexdigest()
 args.output.with_suffix('.proof.json').write_text(json.dumps(proof,indent=2)+'\n',encoding='utf-8',newline='\n')
if __name__=='__main__':main()
