import com.google.gson.GsonBuilder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;

/** 通过官方 1.21.1 的公开模型层和 VertexConsumer 导出部件、原始四角及 UV。
 * 无反射、无私有成员访问、无客户端窗口或世界；仅运行 CPU 模型定义。
 */
public final class EntityLayerReference {
    private static final class Sink implements fbm {
        final List<float[]> vertices = new ArrayList<>();
        public fbm a(float x,float y,float z) { vertices.add(new float[]{x,y,z,0,0}); return this; }
        public fbm a(int r,int g,int b,int a) { return this; }
        public fbm a(float u,float v) { var p=vertices.getLast();p[3]=u;p[4]=v;return this; }
        public fbm a(int u,int v) { return this; }
        public fbm b(int u,int v) { return this; }
        public fbm b(float x,float y,float z) { return this; }
    }
    private static fyk node(fyk root,String path) {
        fyk node=root;
        for(String name:path.split("/")) if(!name.isEmpty()) node=node.b(name);
        return node;
    }
    private static Map<String,Object> part(fyk root,String path) {
        fyk node=node(root,path);
        var out=new TreeMap<String,Object>();
        out.put("pivot",new float[]{node.b,node.c,node.d});
        out.put("rotation",new float[]{node.e,node.f,node.g});
        out.put("scale",new float[]{node.h,node.i,node.j});
        out.put("visible",node.k);out.put("skip_draw",node.l);
        out.put("cubes",new TreeMap<Integer,List<float[]>>());
        return out;
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        ab.a(); akt.a();
        var layers=new TreeMap<String,Object>();
        var references=new TreeMap<String,Object>();
        var poses=new ArrayList<Object>();
        var living=new ArrayList<Object>();
        for(var entry:fyh.a().entrySet()) {
            var root=entry.getValue().a();
            var parts=new TreeMap<String,Map<String,Object>>();
            var baked=new TreeMap<String,Map<Integer,List<float[]>>>();
            parts.put("",part(root,""));
            root.a(new fbi(),(pose,path,index,cube)->{
                String prefix="";
                for(String name:path.split("/")) if(!name.isEmpty()) {
                    prefix+="/"+name;
                    if(!parts.containsKey(prefix)) parts.put(prefix,part(root,prefix));
                }
                var sink=new Sink();
                cube.a(new fbi().c(),sink,0,0,-1);
                if(sink.vertices.size()%4!=0) throw new IllegalStateException("模型面不是四角："+path);
                ((Map<Integer,List<float[]>>)parts.get(path).get("cubes")).put(index,sink.vertices);
                var world=new Sink();cube.a(pose,world,0,0,-1);
                baked.computeIfAbsent(path,ignored->new TreeMap<>()).put(index,world.vertices);
            });
            String key=entry.getKey().a().toString()+"#"+entry.getKey().b();
            layers.put(key,parts);references.put(key,baked);
            if(key.equals("minecraft:creeper#main")) {
                for(float yaw:new float[]{0,90,-37.25f,180}) {
                    var stack=new fbi();
                    stack.a(a.d.rotationDegrees(180-yaw));stack.b(-1,-1,1);stack.a(0,-1.501f,0);
                    var transformed=new TreeMap<String,Map<Integer,List<float[]>>>();
                    root.a(stack,(pose,path,index,cube)->{
                        var sink=new Sink();cube.a(pose,sink,0,0,-1);
                        transformed.computeIfAbsent(path,ignored->new TreeMap<>()).put(index,sink.vertices);
                    });
                    living.add(Map.of("layer",key,"yaw",yaw,"reference",transformed));
                }
            }
            if(Set.of("minecraft:armor_stand#main","minecraft:spider#main","minecraft:frog#main").contains(key)) {
                String child=parts.keySet().stream().filter(p->!p.isEmpty()).findFirst().orElseThrow();
                var overrides=new TreeMap<String,Object>();
                for(String path:List.of("",child)) {
                    var bone=node(root,path);
                    bone.a(1.25f,-2f,3.5f);bone.b(0.3f,-0.8f,1.4f);
                    bone.h=0.75f;bone.i=1.25f;bone.j=0.5f;
                    overrides.put(path,Map.of("pivot",new float[]{bone.b,bone.c,bone.d},
                        "rotation",new float[]{bone.e,bone.f,bone.g},"scale",new float[]{bone.h,bone.i,bone.j}));
                }
                var posed=new TreeMap<String,Map<Integer,List<float[]>>>();
                root.a(new fbi(),(pose,path,index,cube)->{
                    var sink=new Sink();cube.a(pose,sink,0,0,-1);
                    posed.computeIfAbsent(path,ignored->new TreeMap<>()).put(index,sink.vertices);
                });
                poses.add(Map.of("layer",key,"overrides",overrides,"reference",posed));
            }
        }
        var result=new TreeMap<String,Object>();result.put("version","1.21.1");result.put("layers",layers);result.put("reference",references);result.put("poses",poses);result.put("living",living);
        Files.writeString(Path.of(args[0]),new GsonBuilder().disableHtmlEscaping().create().toJson(result));
        System.out.println("导出模型层："+layers.size());
    }
}
