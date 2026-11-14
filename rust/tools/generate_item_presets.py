"""使用准确客户端公开 API 导出完整物品预设和动态着色数据；临时副本用后清理。"""
import argparse, hashlib, json, pathlib, re, tempfile, zipfile
from client_reference import run_source_reference

COMMON={
 '1.21.1':dict(TAG='ub',STACK='cuq',COLORS_TYPE='fhu',LOOKUP_TYPE='jo.a',BOOTSTRAP='ab.a();akt.a();',COLORS='fhu.a(fhq.a())',LOOKUP='var resources=new atx(ass.b,List.of(ats.b()));var base=ka.a(lt.aA);base.c().forEach(e->bind(e.b(),resources));var dynamic=akm.a(resources,base,akm.a);var access=new ka.c(java.util.stream.Stream.concat(base.c(),dynamic.c())).d();access.c().forEach(e->bind(e.b(),resources));lookup=access;',EMPTY='stack.e()',SAVE='(ub)stack.a(lookup)',WRITE='uo.a(tag,(DataOutput)data)',ITEM_KEY='lt.g.b(stack.g()).toString()',COUNT='stack.H()',MAX_COUNT='stack.j()',MAX_DAMAGE='stack.o()',NAME='xb.a.encodeStart(lookup.a(JsonOps.INSTANCE),stack.w()).getOrThrow()',ENGLISH='stack.w().getString()',TOOLTIP='stack.a(cul.b.a,null,cwm.a)',TOOLTIP_LINE='xb.a.encodeStart(lookup.a(JsonOps.INSTANCE),line).getOrThrow()',COLOR='colors.a(stack,i)',BASE_ITEMS='for(var item:lt.g)add(item.w());',CREATIVE='ctb.a(cpn.d.a(),true,lookup);for(var tab:ctb.d())for(var stack:tab.l())add(stack);',EFFECT_REGISTRY='lt.d',EFFECT_COLOR='effect.g()',EFFECT_NAME='effect.d()',EFFECT_NUMERIC='',EFFECT_KEY='lt.d.b(effect).toString()',POTION_REGISTRY='lt.h',POTION_KEY='lt.h.b(potion).toString()',POTION_EFFECT_KEY='lt.d.b(effect.c().a()).toString()',AMPLIFIER='effect.e()',VISIBLE='effect.g()'),
 '1.20.1':dict(TAG='qr',STACK='cfz',COLORS_TYPE='eos',LOOKUP_TYPE='hg.b',BOOTSTRAP='aa.a();acs.a();',COLORS='eos.a(eoo.a())',LOOKUP='var resources=new akq(ajm.b,List.of(new akl().a()));var access=hs.a(jb.ap);access.b().forEach(e->bind(e.b(),resources));lookup=access;',EMPTY='stack.b()',SAVE='stack.b(new qr())',WRITE='rb.a(tag,(DataOutput)data)',ITEM_KEY='jb.i.b(stack.d()).toString()',COUNT='stack.L()',MAX_COUNT='stack.g()',MAX_DAMAGE='stack.l()',NAME='new sw.a().c(stack.y())',ENGLISH='stack.y().getString()',TOOLTIP='stack.a(null,chq.a)',TOOLTIP_LINE='new sw.a().c(line)',COLOR='colors.a(stack,i)',BASE_ITEMS='for(var item:jb.i)add(item.ae_());',CREATIVE='ceg.a(cay.c.a(),true,lookup);for(var tab:ceg.d())for(var stack:tab.l())add(stack);',EFFECT_REGISTRY='jb.e',EFFECT_COLOR='effect.g()',EFFECT_NAME='effect.d()',EFFECT_NUMERIC='e.addProperty("numeric",jb.e.a(effect))',EFFECT_KEY='jb.e.b(effect).toString()',POTION_REGISTRY='jb.j',POTION_KEY='jb.j.b(potion).toString()',POTION_EFFECT_KEY='jb.e.b(effect.c()).toString()',AMPLIFIER='effect.e()',VISIBLE='effect.g()'),
 '1.12.2':dict(TAG='fy',STACK='aip',COLORS_TYPE='bin',LOOKUP_TYPE='Object',BOOTSTRAP='ni.c();',COLORS='bin.a(bik.a())',LOOKUP='',EMPTY='stack.b()',SAVE='stack.a(new fy())',WRITE='gi.a(tag,(DataOutput)data)',ITEM_KEY='ain.g.b(stack.c()).toString()',COUNT='stack.E()',MAX_COUNT='stack.d()',MAX_DAMAGE='stack.k()',NAME='new JsonPrimitive(stack.r())',ENGLISH='stack.r()',TOOLTIP='stack.a(null,akb.a.a)',TOOLTIP_LINE='new JsonPrimitive(line)',COLOR='colors.a(stack,i)',BASE_ITEMS='for(var item:ain.g)add(item.u());',CREATIVE='for(var tab:ahp.a){if(tab!=null&&tab!=ahp.m&&tab!=ahp.n){fi<aip> stacks=fi.a();tab.a(stacks);for(var stack:stacks)add(stack);}}',EFFECT_REGISTRY='uz.b',EFFECT_COLOR='effect.g()',EFFECT_NAME='effect.a()',EFFECT_NUMERIC='e.addProperty("numeric",uz.a(effect))',EFFECT_KEY='uz.b.b(effect).toString()',POTION_REGISTRY='akg.a',POTION_KEY='akg.a.b(potion).toString()',POTION_EFFECT_KEY='uz.b.b(effect.a()).toString()',AMPLIFIER='effect.c()',VISIBLE='effect.e()')
}
def main():
 p=argparse.ArgumentParser(description=__doc__)
 for key in ['client-jar','version-json','libraries','output']:p.add_argument('--'+key,type=pathlib.Path,required=True)
 args=p.parse_args();meta=json.loads(args.version_json.read_text(encoding='utf-8'));version=meta['id'];api=COMMON[version]
 source=pathlib.Path(__file__).with_name('ItemPresetReference.java').read_text(encoding='utf-8')
 api=dict(api)
 api['BIND_METHOD']=''
 if version=='1.21.1':
  api['BIND_METHOD']='static <T>void bind(jz<T> registry,aue resources){var loader=new awv<jm<T>>(id->registry.c(id).map(holder->holder),lu.d(registry.d()));var tags=new HashMap<awu<T>,List<jm<T>>>();for(var entry:loader.b(resources).entrySet())tags.put(awu.a(registry.d(),entry.getKey()),List.copyOf(entry.getValue()));registry.a(tags);}'

 if version=='1.20.1':
  api['BIND_METHOD']='static <T>void bind(hr<T> registry,akx resources){var loader=new anm<he<T>>(id->registry.b(acp.a(registry.c(),id)).map(holder->holder),ann.a(registry.c()));var tags=new HashMap<anl<T>,List<he<T>>>();for(var entry:loader.b(resources).entrySet())tags.put(anl.a(registry.c(),entry.getKey()),List.copyOf(entry.getValue()));registry.a(tags);}'
 if version=='1.12.2':source=source.replace('import com.mojang.serialization.JsonOps;','')
 color_cases=[]
 for id in ['potion','splash_potion','tipped_arrow']:
  if version=='1.21.1':
   payloads=['"minecraft:potion_contents":{potion:"minecraft:water",custom_effects:[{id:"minecraft:speed",amplifier:2},{id:"minecraft:poison",amplifier:1}]}','"minecraft:potion_contents":{custom_effects:[{id:"minecraft:speed",show_particles:0b}]}','"minecraft:potion_contents":{custom_color:1193046}','"minecraft:potion_contents":{custom_effects:[{id:"minecraft:speed",amplifier:2},{id:"minecraft:poison",amplifier:1}]}']
  else:
   payloads=['Potion:"minecraft:water",CustomPotionEffects:[{Id:1b,Amplifier:2b,Duration:100},{Id:19b,Amplifier:1b,Duration:100}]','Potion:"minecraft:water",CustomPotionEffects:[{Id:1b,Duration:100,ShowParticles:0b}]','CustomPotionColor:1193046','CustomPotionEffects:[{Id:1b,Duration:100}]']
  for payload in payloads:color_cases.append((id,payload))
 color_cases.extend([('leather_chestplate','"minecraft:dyed_color":{rgb:1193046}' if version=='1.21.1' else 'display:{color:1193046}'),('filled_map','"minecraft:map_color":1193046' if version=='1.21.1' else 'display:{MapColor:1193046}')])
 for colors in (['[I;]'] if version=='1.21.1' else [])+['[I;1193046]','[I;16711680,255,65280]']:
  color_cases.append(('firework_charge' if version=='1.12.2' else 'firework_star', ('"minecraft:firework_explosion":{shape:"small_ball",colors:'+colors+'}' if version=='1.21.1' else 'Explosion:{Colors:'+colors+'}')))
 statements=[]
 for id,payload in color_cases:
  snbt='{id:"minecraft:'+id+'",'+('count:1,components:{' if version=='1.21.1' else 'Count:1b,tag:{')+payload+'}}'
  parsed={'1.21.1':'cuq.a(lookup,uz.a(SNBT))','1.20.1':'cfz.a(rl.a(SNBT))','1.12.2':'new aip(gp.a(SNBT))'}[version].replace('SNBT',json.dumps(snbt))
  statements.append('cases.add(record('+parsed+'));')
 # 仅查询不依赖世界 / 持有者的属性；不制造时钟或指南针环境。
 extra=[]
 if version=='1.21.1':
  extra=[('diamond_sword','"minecraft:enchantments":{levels:{"minecraft:sharpness":3}}'),('diamond_sword','"minecraft:enchantment_glint_override":1b'),('enchanted_golden_apple','"minecraft:enchantment_glint_override":0b'),('crossbow','"minecraft:charged_projectiles":[{id:"minecraft:arrow",count:1}]'),('crossbow','"minecraft:charged_projectiles":[{id:"minecraft:firework_rocket",count:1}]'),('elytra','"minecraft:damage":431'),('diamond_sword','"minecraft:damage":20'),('light','"minecraft:block_state":{level:"5"}'),('bundle','"minecraft:bundle_contents":[{id:"minecraft:stone",count:32}]')]
 elif version=='1.20.1':
  extra=[('diamond_sword','Enchantments:[{id:"minecraft:sharpness",lvl:3s}],Damage:20'),('crossbow','Charged:1b,ChargedProjectiles:[{id:"minecraft:arrow",Count:1b}]'),('crossbow','Charged:1b,ChargedProjectiles:[{id:"minecraft:firework_rocket",Count:1b}]'),('elytra','Damage:431'),('light','BlockStateTag:{level:"5"}'),('bundle','Items:[{id:"minecraft:stone",Count:32b}]')]
 else:
  extra=[('diamond_sword','ench:[{id:16s,lvl:3s}]')]
 for id,payload in extra:
  snbt='{id:"minecraft:'+id+'",'+('count:1,components:{' if version=='1.21.1' else 'Count:1b,tag:{')+payload+'}}'
  parsed={'1.21.1':'cuq.a(lookup,uz.a(SNBT))','1.20.1':'cfz.a(rl.a(SNBT))','1.12.2':'new aip(gp.a(SNBT))'}[version].replace('SNBT',json.dumps(snbt))
  statements.append('cases.add(record('+parsed+'));')
 root_cases={
  '1.21.1':['{id:"minecraft:diamond_chestplate",count:1,components:{"minecraft:trim":{material:"minecraft:diamond",pattern:"minecraft:sentry"}}}','{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":20,"minecraft:unbreakable":{}}}','{id:"minecraft:stone",count:1,components:{"minecraft:custom_model_data":23}}'],
  '1.20.1':['{id:"minecraft:crossbow",Count:1b,tag:{Charged:0b,ChargedProjectiles:[{id:"minecraft:firework_rocket",Count:1b}]}}','{id:"minecraft:diamond_sword",Count:1b,tag:{Damage:20,Unbreakable:1b}}','{id:"minecraft:stone",Count:1b,tag:{CustomModelData:23}}'],
  '1.12.2':['{id:"minecraft:diamond_sword",Count:1b,Damage:20s}','{id:"minecraft:diamond_sword",Count:1b,Damage:20s,tag:{Unbreakable:1b}}','{id:"minecraft:golden_apple",Count:1b,Damage:1s}']}
 for snbt in root_cases[version]:
  parsed={'1.21.1':'cuq.a(lookup,uz.a(SNBT))','1.20.1':'cfz.a(rl.a(SNBT))','1.12.2':'new aip(gp.a(SNBT))'}[version].replace('SNBT',json.dumps(snbt))
  statements.append('cases.add(record('+parsed+'));')
 api.update({'1.21.1':dict(FOIL='stack.x()',PROPERTY_GET='gps.a(stack,akr.b(key))',PROPERTY_CALL='property.call(stack,null,null,0)'), '1.20.1':dict(FOIL='stack.B()',PROPERTY_GET='fuc.a(stack.d(),new acq(key))',PROPERTY_CALL='property.call(stack,null,null,0)'), '1.12.2':dict(FOIL='stack.u()',PROPERTY_GET='stack.c().a(new nf(key))',PROPERTY_CALL='property.a(stack,null,null)')}[version])
 api['COLOR_CASES']=''.join(statements)
 for key in sorted(api,key=len,reverse=True):source=source.replace(key,api[key])
 if version=='1.12.2':
  catalog=json.loads(pathlib.Path(__file__).resolve().parents[1].joinpath('crates/mcedit2-world/assets/items/1343.json').read_text(encoding='utf-8'))
  extra=''.join('for(int damage:new int[]{'+','.join(item.get('variants',{}))+'})add(new aip(ain.b('+json.dumps(id)+'),1,damage));' for id,item in catalog['items'].items() if item.get('variants'))
  source=source.replace(api['BASE_ITEMS'],api['BASE_ITEMS']+extra)
 source=source.replace('VERSION',json.dumps(version))
 base=pathlib.Path(__file__).resolve().parents[1]/'target/item-reference'
 with tempfile.TemporaryDirectory(prefix='presets-',dir=base) as temporary:
  proof=run_source_reference(args,'ItemPresetReference',source,pathlib.Path(temporary),version)
 if version=='1.12.2':
  with zipfile.ZipFile(args.client_jar) as jar:
   entries={}
   for line in jar.read('assets/minecraft/lang/en_us.lang').decode('utf-8').splitlines():
    if line and not line.startswith('#') and '=' in line:
     key,text=line.split('=',1);entries.setdefault(text,[]).append(key)
  def component(text):
   text=re.sub('§[0-9a-fk-or]','',text)
   keys=entries.get(text,[])
   return {'translate':keys[0],'fallback':text} if len(keys)==1 else text
  data=json.loads(args.output.read_text(encoding='utf-8'))
  for preset in data['presets']+data['color_cases']:
   preset['name']=component(preset['name']);preset['tooltip']=[component(t) for t in preset['tooltip']]
  args.output.write_text(json.dumps(data,ensure_ascii=False,separators=(',',':')),encoding='utf-8',newline='\n')
  proof['output_sha256']=hashlib.sha256(args.output.read_bytes()).hexdigest()
 args.output.with_suffix('.proof.json').write_text(json.dumps(proof,indent=2)+'\n',encoding='utf-8',newline='\n')
if __name__=='__main__':main()
