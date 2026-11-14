"""导出保存状态下的碰撞目录及移动活塞原生谓词参照；不启动窗口，不用反射。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tempfile
import zipfile

from client_reference import run_source_reference
from generate_spawn_catalog import API, mapping_classes, resolved_tags


SOURCE = r'''
import com.google.gson.*;
import java.nio.file.*;
import java.util.*;

@SuppressWarnings({"rawtypes", "unchecked"})
public final class SpawnContextReference {
    private static final class World implements @getter@ {
        @block_entity@ entity;
        boolean read;
        public @block_entity@ c_(@position@ pos) { read=true; return entity; }
        public @state@ a_(@position@ pos) { read=true; return @empty@.a_(pos); }
        public @fluid@ b_(@position@ pos) { read=true; return @empty@.b_(pos); }
        public int @height@() { read=true; return @empty@.@height@(); }
        public int @minimum@() { read=true; return @empty@.@minimum@(); }
    }
    private static String property(@property@ key, Comparable value) { return key.a(value); }
    private static JsonObject state(@state@ state) {
        JsonObject result=new JsonObject(), properties=new JsonObject();
        result.addProperty("name", @registry@.b(state.b()).toString());
        for(var entry:state.C().entrySet()) properties.addProperty(entry.getKey().f(), property(entry.getKey(),entry.getValue()));
        result.add("properties",properties); return result;
    }
    private static JsonArray boxes(@shape@ shape) {
        var result=new JsonArray();
        for(var box:shape.@boxes@()) {
            var row=new JsonArray();
            for(double coordinate:new double[]{box.a,box.b,box.c,box.d,box.e,box.f}) row.add(coordinate);
            result.add(row);
        }
        return result;
    }
    private static void bind(JsonObject definitions) {
        Map bindings=new HashMap();
        for(var entry:definitions.entrySet()) {
            String id=entry.getKey(); var tag=@tags@.a(@registries@,@resource@); List holders=new ArrayList();
            for(var element:entry.getValue().getAsJsonArray()) {
                id=element.getAsString(); holders.add(@registry@.b(@key@.a(@registries@,@resource@)).orElseThrow());
            }
            bindings.put(tag,holders);
        }
        @registry@.a(bindings);
    }
    public static void main(String[] args) throws Exception {
        @bootstrap@
        @lookup@
        bind(JsonParser.parseString(Files.readString(Path.of(@input@))).getAsJsonObject());
        var world=new World(); var shapes=new JsonArray(); var shapeIds=new LinkedHashMap<String,Integer>();
        var blocks=new TreeMap<String,JsonObject>(); var representatives=new LinkedHashMap<String,@state@>();
        var moving=@registry@.a(@moving_id@).@default_state@();
        var shulkers=new JsonArray();
        for(@block@ block:@registry@) {
            String kind=block instanceof @shulker_block@?"shulker":block instanceof @moving_block@?"moving_piston":block instanceof @piston_block@?"piston":"static";
            JsonObject definition=new JsonObject();definition.addProperty("kind",kind);
            JsonArray properties=new JsonArray(),states=new JsonArray();
            for(var key:block.@default_state@().C().keySet())properties.add(key.f());
            definition.add("properties",properties);definition.add("states",states);
            for(@state@ value:block.l().a()) {
                world.entity=null; world.read=false;
                var shape=value.k(world,@zero@); var geometry=boxes(shape);
                if(world.read && !(kind.equals("moving_piston")||kind.equals("shulker"))) throw new IllegalStateException("未解释的碰撞世界读取："+value);
                boolean offset=value.q();
                if(offset && !geometry.isEmpty()) {
                    double minX=Double.POSITIVE_INFINITY,minZ=minX,maxX=Double.NEGATIVE_INFINITY,maxZ=maxX;
                    for(var box:geometry){var a=box.getAsJsonArray();minX=Math.min(minX,a.get(0).getAsDouble());minZ=Math.min(minZ,a.get(2).getAsDouble());maxX=Math.max(maxX,a.get(3).getAsDouble());maxZ=Math.max(maxZ,a.get(5).getAsDouble());}
                    if(!(maxX-minX<1.0 || maxZ-minZ<1.0)) throw new IllegalStateException("偏移碰撞可能覆盖完整顶面："+value);
                }
                String key=geometry.toString();Integer shapeId=shapeIds.get(key);
                if(shapeId==null){shapeId=shapes.size();shapeIds.put(key,shapeId);shapes.add(geometry);}
                var row=new JsonArray();var values=new JsonArray();
                for(var entry:value.C().entrySet())values.add(property(entry.getKey(),entry.getValue()));
                row.add(values);row.add(shapeId);row.add(offset);states.add(row);
                if(!kind.equals("moving_piston"))representatives.putIfAbsent(kind+":"+shapeId+":"+offset,value);
                if(kind.equals("shulker")) {
                    world.entity=new @shulker_entity@(@zero@,value);
                    var record=new JsonObject();record.add("state",state(value));
                    record.addProperty("flags",(value.a(world,@zero@,@zombie@)?1:0)|(@spawner@.a(world,@zero@,value,value.u(),@zombie@)?2:0));
                    shulkers.add(record);
                }
            }
            blocks.put(@registry@.b(block).toString(),definition);
        }
        float[] progresses={-0.5f,0f,1.0e-8f,0.25f,0.5f,0.75f,0.99999994f,1f,1.0000001f,1.5f};
        var motions=new JsonArray();
        for(var direction:@direction@.values())for(boolean extending:new boolean[]{false,true})for(float progress:progresses) {
            var motion=new JsonObject();motion.addProperty("facing",direction.d());motion.addProperty("extending",extending);motion.addProperty("progress",progress);motions.add(motion);
        }
        var references=new JsonArray();boolean sourceStatic=false;
        for(var entry:representatives.entrySet()) {
            var moved=entry.getValue();boolean piston=moved.b() instanceof @piston_block@;
            boolean sourceCases=piston||!sourceStatic;sourceStatic=true;
            JsonArray expected=new JsonArray(),expectedSource=new JsonArray();
            for(var element:motions) {
                var motion=element.getAsJsonObject();
                for(boolean source:new boolean[]{false,true}) {
                    if(source&&!sourceCases)continue;
                    var tag=new @compound@();tag.a("blockState",@nbt_utils@.a(moved));
                    tag.a("facing",motion.get("facing").getAsInt());tag.a("extending",motion.get("extending").getAsBoolean());tag.a("progress",motion.get("progress").getAsFloat());tag.a("source",source);
                    var entity=new @piston_entity@(@zero@,moving);@load@
                    world.entity=entity;
                    int flags=(moving.a(world,@zero@,@zombie@)?1:0)|(@spawner@.a(world,@zero@,moving,moving.u(),@zombie@)?2:0);
                    (source?expectedSource:expected).add(flags);
                }
            }
            var reference=new JsonObject();reference.add("state",state(moved));reference.add("expected",expected);reference.add("source_expected",expectedSource);references.add(reference);
        }
        var output=new JsonObject();output.addProperty("data_version",@data_version@);output.add("shapes",shapes);output.add("blocks",new Gson().toJsonTree(blocks));
        output.add("moving_state",state(moving));output.add("motions",motions);output.add("references",references);output.add("shulkers",shulkers);
        Files.writeString(Path.of(args[0]),new Gson().toJson(output)+"\n");
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("client-jar", "version-json", "libraries", "mappings"):
        parser.add_argument("--"+key, type=Path, required=True)
    args = parser.parse_args()
    version = json.loads(args.version_json.read_text(encoding="utf-8"))["id"]
    classes = mapping_classes(args.mappings)
    api = dict(API[version])
    for key, name in {
        "getter":"world.level.BlockGetter", "block_entity":"world.level.block.entity.BlockEntity",
        "position":"core.BlockPos", "fluid":"world.level.material.FluidState", "shape":"world.phys.shapes.VoxelShape",
        "direction":"core.Direction", "compound":"nbt.CompoundTag", "nbt_utils":"nbt.NbtUtils",
        "shulker_block":"world.level.block.ShulkerBoxBlock", "shulker_entity":"world.level.block.entity.ShulkerBoxBlockEntity",
        "moving_block":"world.level.block.piston.MovingPistonBlock", "piston_block":"world.level.block.piston.PistonBaseBlock",
        "piston_entity":"world.level.block.piston.PistonMovingBlockEntity",
    }.items():
        api[key] = classes["net.minecraft."+name]
    api.update(boxes="e" if version=="1.21.1" else "d", default_state="o" if version=="1.21.1" else "n",
               lookup="jo.a lookup=ka.a(lt.aA);" if version=="1.21.1" else "",
               load="entity.c(tag,lookup);" if version=="1.21.1" else "entity.a(tag);",
               moving_id='akr.a("minecraft:moving_piston")' if version=="1.21.1" else 'new acq("minecraft:moving_piston")')
    root=Path(__file__).resolve().parents[1]
    with zipfile.ZipFile(args.client_jar) as jar:
        prefix="data/minecraft/tags/"+api["tag_path"]+"/"
        tags={"minecraft:"+name[len(prefix):-5]:json.loads(jar.read(name)) for name in jar.namelist() if name.startswith(prefix) and name.endswith(".json")}
    with tempfile.TemporaryDirectory(prefix="spawn-context-",dir=root/"target") as temporary:
        work=Path(temporary);tag_path=work/"tags.json";tag_path.write_text(json.dumps(resolved_tags(tags)),encoding="utf-8")
        source=SOURCE
        for key,value in dict(api,input=json.dumps(str(tag_path)),data_version=str(3955 if version=="1.21.1" else 3465)).items():source=source.replace("@"+key+"@",value)
        if re.search(r"@[a-z_]+@",source):raise ValueError("尚未绑定的原生上下文 API")
        args.output=work/"context.json"
        proof=run_source_reference(args,"SpawnContextReference",source,work,version)
        output=root/"crates/mcedit2-world/assets"/("spawn-context-"+version+".json")
        output.write_bytes(args.output.read_bytes())
        proof.update(source_sha256=hashlib.sha256(source.encode()).hexdigest(),mappings_sha256=hashlib.sha256(args.mappings.read_bytes()).hexdigest(),
                     scope="空碰撞上下文、保存后关闭的潜影盒、实际载入 NBT 的移动活塞支撑与身体谓词；没有运行时动画或游戏窗口")
        output.with_suffix(".proof.json").write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
        data=json.loads(output.read_bytes())
        print(version,len(data["shapes"]),"种形状",len(data["references"]),"组代表状态",sum(len(r["expected"])+len(r["source_expected"]) for r in data["references"]),"组原生移动谓词")


if __name__ == "__main__":
    main()
