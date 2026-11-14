"""导出准确现代客户端的生成表面 / 身体空间属性、原生标签及维度光照定义。无反射或窗口。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import zipfile

from client_reference import run_source_reference


API = {
    "1.21.1": dict(bootstrap="ab.a(); akt.a();", block="dfy", state="dtc",
                   registry="lt.e", empty="dcl.a", zero="jd.c", zombie="bsx.bu",
                   spawner="ddh", property="duf", tags="awu", registries="lu.f",
                   resource="akr.a(id)", key="akq", dynamic="p", tag_path="block",
                   height="J_", minimum="I_"),
    "1.20.1": dict(bootstrap="aa.a(); acs.a();", block="cpn", state="dcb",
                   registry="jb.f", empty="cmb.a", zero="gu.b", zombie="bfn.bp",
                   spawner="cmx", property="dde", tags="anl", registries="jc.e",
                   resource="new acq(id)", key="acp", dynamic="o", tag_path="blocks",
                   height="D_", minimum="C_"),
}


def mapping_classes(path):
    return dict(line[:-1].split(" -> ") for line in path.read_text(encoding="utf-8").splitlines()
                if line and not line.startswith((" ", "#")))


def native_methods(jar, classes):
    """保留实际字节码观察的摘要；不是 REA 反编译或原始源码。"""
    result = {}
    for name in ("net.minecraft.world.entity.monster.Monster",
                 "net.minecraft.world.level.NaturalSpawner",
                 "net.minecraft.world.level.lighting.SkyLightSectionStorage"):
        output = subprocess.check_output(
            ["javap", "-c", "-p", "-classpath", str(jar), classes[name]], text=True)
        result[name] = {"class": classes[name], "javap_sha256": hashlib.sha256(output.encode()).hexdigest()}
    return result


SOURCE = r'''
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.nio.file.*;
import java.util.*;

/** 直接调用准确客户端 API；标签来自该 JAR 的实际数据定义。 */
@SuppressWarnings({"rawtypes", "unchecked"})
public final class SpawnPropertyReference {
    /** 记录属性求值实际读取的世界合同，不把空环境的动态结果当作完整状态常量。 */
    private static final class Probe implements @getter@ {
        boolean read;
        public @block_entity@ c_(@position@ pos) { read = true; return @empty@.c_(pos); }
        public @state@ a_(@position@ pos) { read = true; return @empty@.a_(pos); }
        public @fluid@ b_(@position@ pos) { read = true; return @empty@.b_(pos); }
        public int @height@() { read = true; return @empty@.@height@(); }
        public int @minimum@() { read = true; return @empty@.@minimum@(); }
    }
    private static String property(@property@ key, Comparable value) { return key.a(value); }
    private static void bind(JsonObject definitions, boolean withoutRuntimeTags) {
        Map bindings = new HashMap();
        for (var entry : definitions.entrySet()) {
            String id = entry.getKey();
            var tag = @tags@.a(@registries@, @resource@);
            List holders = new ArrayList();
            if (!withoutRuntimeTags || !(id.equals("minecraft:fire") || id.equals("minecraft:prevent_mob_spawning_inside"))) {
                for (var element : entry.getValue().getAsJsonArray()) {
                    id = element.getAsString();
                    holders.add(@registry@.b(@key@.a(@registries@, @resource@)).orElseThrow());
                }
            }
            bindings.put(tag, holders);
        }
        @registry@.a(bindings);
    }
    public static void main(String[] args) throws Exception {
        @bootstrap@
        JsonObject definitions = JsonParser.parseString(Files.readString(Path.of(@input@))).getAsJsonObject();
        bind(definitions, false);
        var all = new ArrayList<@state@>();
        var flags = new ArrayList<Integer>();
        var probe = new Probe();
        for (@block@ block : @registry@) {
            for (@state@ state : block.l().a()) {
                all.add(state);
                probe.read = false;
                int bits = (state.a(probe, @zero@, @zombie@) ? 1 : 0) | (probe.read ? 8 : 0);
                probe.read = false;
                bits |= (@spawner@.a(probe, @zero@, state, state.u(), @zombie@) ? 4 : 0)
                    | (probe.read ? 16 : 0) | (block.@dynamic@() ? 32 : 0);
                flags.add(bits);
            }
        }
        bind(definitions, true);
        var blocks = new TreeMap<String, JsonObject>();
        for (int index = 0; index < all.size(); index++) {
            var state = all.get(index);
            probe.read = false;
            int bits = flags.get(index)
                | (@spawner@.a(probe, @zero@, state, state.u(), @zombie@) ? 2 : 0) | (probe.read ? 16 : 0);
            String id = @registry@.b(state.b()).toString();
            var block = blocks.get(id);
            if (block == null) {
                block = new JsonObject();
                var properties = new JsonArray();
                for (var key : state.C().keySet()) properties.add(key.f());
                block.add("properties", properties); block.add("states", new JsonArray());
                blocks.put(id, block);
            }
            var values = new JsonArray();
            for (var value : state.C().entrySet()) values.add(property(value.getKey(), value.getValue()));
            var record = new JsonArray(); record.add(values); record.add(bits);
            block.getAsJsonArray("states").add(record);
        }
        JsonObject output = new JsonObject();
        output.addProperty("version", @version@);
        output.addProperty("data_version", @data_version@);
        output.add("blocks", new Gson().toJsonTree(blocks));
        JsonArray providers = new JsonArray();
        for (String input : @providers@) {
            var json = JsonParser.parseString(input);
            var decoded = @provider@.c.parse(JsonOps.INSTANCE, json);
            var row = new JsonObject(); row.add("input", json);
            row.addProperty("accepted", decoded.result().isPresent());
            if (decoded.result().isPresent()) {
                var value = decoded.result().orElseThrow();
                row.addProperty("min", value.a()); row.addProperty("max", value.b());
            } else { row.addProperty("error", decoded.error().orElseThrow().toString()); }
            providers.add(row);
        }
        output.add("light_providers", providers);
        Files.writeString(Path.of(args[0]), new Gson().toJson(output) + "\n");
    }
}
'''


def resolved_tags(definitions):
    resolved = {}
    active = set()

    def resolve(name):
        if name in active:
            raise ValueError("原生方块标签循环：" + name)
        if name in resolved:
            return resolved[name]
        active.add(name)
        values = set()
        for entry in definitions[name]["values"]:
            value = entry if isinstance(entry, str) else entry["id"]
            required = isinstance(entry, str) or entry.get("required", True)
            if value.startswith("#"):
                key = value[1:] if ":" in value else "minecraft:" + value[1:]
                if not required and key not in definitions:
                    continue
                values.update(resolve(key))
            else:
                values.add(value if ":" in value else "minecraft:" + value)
        active.remove(name)
        resolved[name] = sorted(values)
        return resolved[name]

    for name in definitions:
        resolve(name)
    return resolved


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("client-jar", "version-json", "libraries", "mappings"):
        parser.add_argument("--" + key, type=Path, required=True)
    args = parser.parse_args()
    version = json.loads(args.version_json.read_text(encoding="utf-8"))["id"]
    api = API[version]
    root = Path(__file__).resolve().parents[1]
    assets = root / "crates/mcedit2-world/assets"
    data_version = {"1.20.1": 3465, "1.21.1": 3955}[version]
    classes = mapping_classes(args.mappings)
    api = dict(api, **{key: classes["net.minecraft." + name] for key, name in {
        "getter": "world.level.BlockGetter", "block_entity": "world.level.block.entity.BlockEntity",
        "position": "core.BlockPos", "fluid": "world.level.material.FluidState",
        "provider": "util.valueproviders.IntProvider",
    }.items()})
    providers = [0, 7, -1, 16]
    for kind, fields in [
        ("constant", {"value": 4}),
        ("uniform", {"min_inclusive": 2, "max_inclusive": 7}),
        ("biased_to_bottom", {"min_inclusive": 2, "max_inclusive": 7}),
        ("clamped", {"source": 12, "min_inclusive": 2, "max_inclusive": 7}),
        ("clamped_normal", {"mean": 4., "deviation": 2., "min_inclusive": 2, "max_inclusive": 7}),
        ("weighted_list", {"distribution": [{"data": 2, "weight": 1}, {"data": 7, "weight": 3}]}),
        ("uniform", {"min_inclusive": 7, "max_inclusive": 2}),
    ]:
        providers.extend([dict(type="minecraft:" + kind, **fields),
                          dict(type="minecraft:" + kind, value=fields)])
    providers.extend([dict(type="minecraft:clamped", source={"type": "minecraft:uniform", "min_inclusive": 0, "max_inclusive": 15}, min_inclusive=2, max_inclusive=7),
                      dict(type="unknown:light", value=1)])
    definitions, dimensions = {}, {}
    with zipfile.ZipFile(args.client_jar) as jar:
        tag_prefix = "data/minecraft/tags/" + api["tag_path"] + "/"
        for name in sorted(jar.namelist()):
            if name.startswith(tag_prefix) and name.endswith(".json"):
                definitions["minecraft:" + name[len(tag_prefix):-5]] = json.loads(jar.read(name))
            if name.startswith("data/minecraft/dimension_type/") and name.endswith(".json"):
                dimensions["minecraft:" + name.removeprefix("data/minecraft/dimension_type/")[:-5]] = json.loads(jar.read(name))
    with tempfile.TemporaryDirectory(prefix="spawn-reference-", dir=root / "target") as temporary:
        work = Path(temporary)
        tags = work / "block-tags.json"
        tags.write_text(json.dumps(resolved_tags(definitions)), encoding="utf-8")
        source = SOURCE
        provider_source = "new String[]{" + ",".join(json.dumps(json.dumps(value)) for value in providers) + "}"
        for name, value in dict(api, input=json.dumps(str(tags)), version=json.dumps(version), data_version=str(data_version), providers=provider_source).items():
            source = source.replace("@" + name + "@", value)
        if re.search(r"@[a-z_]+@", source):
            raise ValueError("生成器有未绑定的原生接口")
        args.output = work / "spawn.json"
        proof = run_source_reference(args, "SpawnPropertyReference", source, work, version)
        output = assets / ("spawn-properties-" + version + ".json")
        output.write_bytes(args.output.read_bytes())
    for prefix, values in [("spawn-block-tags", definitions), ("spawn-dimensions", dimensions)]:
        (assets / (prefix + "-" + version + ".json")).write_text(
            json.dumps(values, separators=(",", ":")) + "\n", encoding="utf-8")
    proof.update(source_sha256=hashlib.sha256(source.encode()).hexdigest(),
                 mappings_sha256=hashlib.sha256(args.mappings.read_bytes()).hexdigest(),
                 native_methods=native_methods(args.client_jar, classes),
                 flag_bits={"support": 1, "body_without_world_tags": 2, "body_with_native_tags": 4,
                            "support_reads_world": 8, "body_reads_world": 16, "dynamic_shape": 32},
                 scope="僵尸所代表的普通步行敌对生物表面 / 身体规则；动态形状单独标记；标签由实际来源解释")
    output.with_suffix(".proof.json").write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    data = json.loads(output.read_text(encoding="utf-8"))
    print(version, len(data["blocks"]), sum(len(b["states"]) for b in data["blocks"].values()), "个原生状态")


if __name__ == "__main__":
    main()
