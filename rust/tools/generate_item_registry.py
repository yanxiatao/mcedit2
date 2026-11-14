"""从准确官方客户端的公开 API 导出物品清单；不使用反射、不启动窗口。"""
import argparse, hashlib, json, os, shutil, subprocess, tempfile, zipfile
from pathlib import Path

API = {
 "1.12.2": ("ni.c();", "ain", "ain.g", "item.u()", 'item.a(stack)+".name"', "item.j()", "item.l()", "bin.a(bik.a())"),
 "1.20.1": ("aa.a(); acs.a();", "cfu", "jb.i", "item.ae_()", "item.j(stack)", "stack.g()", "stack.l()", "eos.a(eoo.a())"),
 "1.21.1": ("ab.a(); akt.a();", "cul", "lt.g", "item.w()", "item.h(stack)", "stack.j()", "stack.o()", "fhu.a(fhq.a())"),
}

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument('--version-json',type=Path,required=True)
 parser.add_argument('--client',type=Path,required=True)
 parser.add_argument('--libraries',type=Path,required=True)
 parser.add_argument('--output',type=Path,required=True)
 args=parser.parse_args()
 meta=json.loads(args.version_json.read_text(encoding='utf-8')); version=meta['id']
 bootstrap,item,registry,stack,key,count,damage,colors=API[version]
 sha=hashlib.sha1(args.client.read_bytes()).hexdigest()
 if sha!=meta['downloads']['client']['sha1']: raise ValueError('客户端摘要不符')
 dependencies=[]
 for lib in meta['libraries']:
  artifact=lib.get('downloads',{}).get('artifact')
  if artifact:
   path=args.libraries/artifact['path']
   if path.is_file(): dependencies.append(str(path.resolve()))
 source='import com.google.gson.*;\nimport java.nio.file.*;\npublic class ItemRegistryExport {\n public static void main(String[] args) throws Exception {\n  var stderr=System.err;\n  try { BOOTSTRAP } finally { System.setErr(stderr); }\n  COLORS\n  var items=new JsonObject();\n  for (ITEM item : REGISTRY) {\n   var stack=STACK;\n   var data=new JsonObject();\n   data.addProperty("name",KEY);\n   data.addProperty("max_count",COUNT);\n   data.addProperty("max_damage",DAMAGE);\n   TINT\n   items.add(REGISTRY.b(item).toString(),data);\n  }\n  var result=new JsonObject();\n  result.addProperty("version",VERSION);\n  result.addProperty("client_sha1",SHA);\n  result.add("items",items);\n  Files.writeString(Path.of(args[0]),new GsonBuilder().setPrettyPrinting().create().toJson(result));\n }\n}\n'
 if version=='1.12.2':
  source=source.replace('TINT', '''var held=new JsonObject();
   held.addProperty("block_entity",aow.a(item).t().i()==atj.c);
   held.addProperty("full_3d",item.D_());held.addProperty("rotate",item.F_());
   data.add("legacy_held",held);
   TINT''')
  source=source.replace('public static void main','''static java.util.Map<ain,java.util.Map<Integer,String>> models=new java.util.LinkedHashMap<>();
 // 在真正查询模型的位置返回资源 ID，不加载纹理或创建渲染窗口。
 static final class ModelLocation extends RuntimeException {
  final String name;ModelLocation(cgd model) {name=model.toString().split("#",2)[0];}
 }
 static final class ModelLookup extends cgc {
  ModelLookup() {super(null);}
  @Override public cfy a(cgd model) {throw new ModelLocation(model);}
  @Override public cfy a() {throw new IllegalStateException("物品没有任何模型绑定");}
 }
 static final class Bindings extends bzw {
  Bindings(bin colors) {super(null,new ModelLookup(),colors);}
  @Override protected void a(ain item,int damage,String model) {
   models.computeIfAbsent(item,k->new java.util.TreeMap<>()).put(damage,model);
  }
 }
 public static void main''')
  source=source.replace('var items=new JsonObject();','var bindings=new Bindings(colors); var items=new JsonObject();')
  source=source.replace('items.add(REGISTRY.b(item).toString(),data);','''if(!models.containsKey(item) && !REGISTRY.b(item).toString().equals("minecraft:air")) {
    try {bindings.a().a(stack);throw new IllegalStateException("模型查询没有经过资源入口");}
    catch(ModelLocation location) {models.computeIfAbsent(item,k->new java.util.TreeMap<>()).put(0,location.name);}
   }
   if(REGISTRY.b(item).toString().equals("minecraft:bed") || REGISTRY.b(item).toString().equals("minecraft:banner")) {
    fi<aip> choices=fi.a();item.a(item.b(),choices);
    for(aip choice:choices) {
     try {bindings.a().a(choice);throw new IllegalStateException("变体没有经过模型资源查询");}
     catch(ModelLocation location) {models.get(item).put(choice.j(),location.name);}
    }
   }
   var variants=new JsonObject();
   for(var binding:models.getOrDefault(item,java.util.Map.of()).entrySet()) {
    var variant=new JsonObject();var sample=new aip(item,1,binding.getKey());
    variant.addProperty("model",binding.getValue());
    variant.addProperty("name",item.a(sample)+".name");
    var variantTint=new JsonArray();for(int i=0;i<5;i++)variantTint.add(colors.a(sample,i));variant.add("tints",variantTint);
    variants.add(binding.getKey().toString(),variant);
   }
   data.add("variants",variants);
   items.add(REGISTRY.b(item).toString(),data);''')
 for before,after in [('BOOTSTRAP',bootstrap),('COLORS',('var colors='+colors+';') if colors else ''),('ITEM',item),('REGISTRY',registry),('STACK',stack),('KEY',key),('COUNT',count),('DAMAGE',damage),('TINT','var tint=new JsonArray(); for(int i=0;i<5;i++) tint.add(colors.a(stack,i)); data.add("tints",tint);' if colors else ''),('VERSION',json.dumps(version)),('SHA',json.dumps(sha))]: source=source.replace(before,after)
 args.output.parent.mkdir(parents=True,exist_ok=True)
 with tempfile.TemporaryDirectory(prefix='items-',dir=args.output.parent) as temporary:
  work=Path(temporary).resolve(); analysis=work/'analysis.jar'
  with zipfile.ZipFile(args.client) as original,zipfile.ZipFile(analysis,'w') as target:
   for entry in original.infolist():
    upper=entry.filename.upper()
    if upper.startswith('META-INF/') and upper.endswith(('.SF','.RSA','.DSA','.EC')): continue
    target.writestr(entry,original.read(entry))
  with zipfile.ZipFile(args.client) as original,zipfile.ZipFile(analysis) as target:
   for name in target.namelist():
    if original.read(name)!=target.read(name): raise ValueError('分析副本成员改变：'+name)
  java, javac = shutil.which('java'), shutil.which('javac')
  if not java or not javac: raise RuntimeError('需要已有 JDK，不自动安装')
  logging=work/'logging.xml'
  logging.write_text('<Configuration><Appenders><Console name="Console" target="SYSTEM_ERR"><PatternLayout pattern="%level: %msg%n"/></Console></Appenders><Loggers><Root level="warn"><AppenderRef ref="Console"/></Root></Loggers></Configuration>',encoding='utf-8')
  classpath=os.pathsep.join([str(analysis)]+dependencies)
  code=work/'ItemRegistryExport.java';code.write_text(source,encoding='utf-8')
  subprocess.run([javac,'-proc:none','-encoding','UTF-8','-cp',classpath,'-d',str(work),str(code)],check=True,cwd=work)
  subprocess.run([java,'-Dlog4j2.formatMsgNoLookups=true','-Dlog4j.configurationFile='+logging.as_uri(),'-cp',str(work)+os.pathsep+classpath,'ItemRegistryExport',str(args.output.resolve())],check=True,cwd=work)
 result=json.loads(args.output.read_text());print(version,len(result['items']),'种物品；临时副本和类已清理')

if __name__=='__main__':main()
