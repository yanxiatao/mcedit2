import com.google.gson.GsonBuilder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;

/** 官方客户端公开模型的 CPU renderToBuffer 数值参照；不创建世界或实体。 */
public final class NpcModelReference {
    private static final class Sink implements fbm {
        final List<float[]> vertices = new ArrayList<>();
        public fbm a(float x,float y,float z) { vertices.add(new float[]{x,y,z,0,0}); return this; }
        public fbm a(int r,int g,int b,int a) { return this; }
        public fbm a(float u,float v) { var p=vertices.getLast(); p[3]=u;p[4]=v;return this; }
        public fbm a(int u,int v) { return this; }
        public fbm b(int u,int v) { return this; }
        public fbm b(float x,float y,float z) { return this; }
    }
    private static fyk root(String key) {
        return fyh.a().entrySet().stream().filter(e -> (e.getKey().a()+"#"+e.getKey().b()).equals(key))
            .findFirst().orElseThrow().getValue().a();
    }
    private static fyk part(fyk root,String path) {
        fyk p=root; for(String s:path.split("/")) if(!s.isEmpty()) p=p.b(s); return p;
    }
    private static void rotation(fyk root,Map<String,Object> overrides,String name,float x,float y,float z) {
        var p=part(root,name);p.e=x;p.f=y;p.g=z;
        overrides.put("/"+name,Map.of("rotation",new float[]{x,y,z},"visible",p.k));
    }
    private static void visible(fyk root,Map<String,Object> overrides,String name,boolean value) {
        var p=part(root,name);p.k=value;
        overrides.put("/"+name,Map.of("rotation",new float[]{p.e,p.f,p.g},"visible",p.k));
    }
    private static float rad(float degrees) { return degrees * ((float)Math.PI / 180.0f); }
    private static Map<String,Object> build(String id,String layer,boolean young,float yaw,boolean hat,boolean arms,boolean base,boolean posed) {
        var root=root(layer); fvk<?> model;
        if(layer.equals("minecraft:sheep#main")) model=new fxa<>(root);
        else if(layer.equals("minecraft:sheep#fur")) model=new fwz<>(root);
        else if(layer.equals("minecraft:villager#main")) model=new fxv<>(root);
        else if(layer.equals("minecraft:armor_stand#main")) model=new ful(root);
        else throw new IllegalArgumentException(layer);
        model.e=young;
        var overrides=new TreeMap<String,Object>();
        if(layer.startsWith("minecraft:sheep")) {
            rotation(root,overrides,"head",0.35f,-0.22f,0);
            for(var name:List.of("left_front_leg","right_front_leg","left_hind_leg","right_hind_leg"))
                rotation(root,overrides,name,name.startsWith("left")?0.4f:-0.4f,0,0);
        } else if(layer.startsWith("minecraft:villager")) {
            rotation(root,overrides,"head",0.2f,-0.3f,0);
            ((fxv<?>)model).a(hat);
            for(var name:List.of("head","head/hat","head/hat/hat_rim")) {
                var p=part(root,name);overrides.put("/"+name,Map.of("rotation",new float[]{p.e,p.f,p.g},"visible",p.k));
            }
        } else {
            float[][] poses=posed ? new float[][]{{23,-31,17},{12,25,-8},{-40,12,-23},{-32,-19,22},{8,15,-9},{-11,-16,13}}
                : new float[][]{{0,0,0},{0,0,0},{-10,0,-10},{-15,0,10},{-1,0,-1},{1,0,1}};
            var names=List.of("head","body","left_arm","right_arm","left_leg","right_leg");
            for(int i=0;i<names.size();i++) rotation(root,overrides,names.get(i),rad(poses[i][0]),rad(poses[i][1]),rad(poses[i][2]));
            rotation(root,overrides,"hat",rad(poses[0][0]),rad(poses[0][1]),rad(poses[0][2]));
            // ArmorStandArmorModel.setupAnim 使用 ModelPart.copyFrom(head)，同时复制轴心。
            var head=part(root,"head");var hatPart=part(root,"hat");hatPart.a(head);
            overrides.put("/hat",Map.of("rotation",new float[]{hatPart.e,hatPart.f,hatPart.g},
                "pivot",new float[]{hatPart.b,hatPart.c,hatPart.d},"visible",hatPart.k));
            for(var name:List.of("left_body_stick","right_body_stick","shoulder_stick"))
                rotation(root,overrides,name,rad(poses[1][0]),rad(poses[1][1]),rad(poses[1][2]));
            rotation(root,overrides,"base_plate",0,rad(-yaw),0);
            visible(root,overrides,"left_arm",arms);visible(root,overrides,"right_arm",arms);visible(root,overrides,"base_plate",base);
        }
        float extra=layer.startsWith("minecraft:villager")?(young?0.46875f:0.9375f):1f;
        var stack=new fbi();stack.a(a.d.rotationDegrees(180-yaw));stack.b(-1,-1,1);
        stack.b(extra,extra,extra);stack.a(0,-1.501f,0);
        var sink=new Sink();model.a(stack,sink,0,0,-1);
        if(sink.vertices.size()%4!=0) throw new IllegalStateException("输出不是四角面: "+id);
        var out=new LinkedHashMap<String,Object>();
        out.put("id",id);out.put("layer",layer);out.put("young",young);out.put("yaw",yaw);out.put("scale",extra);
        out.put("overrides",overrides);out.put("vertices",sink.vertices);return out;
    }
    public static void main(String[] args) throws Exception {
        ab.a();akt.a();var cases=new ArrayList<Object>();
        for(String layer:List.of("minecraft:sheep#main","minecraft:sheep#fur")) for(boolean young:List.of(false,true))
            cases.add(build(layer+(young?"-young":"-adult"),layer,young,37.25f,true,true,true,true));
        for(boolean young:List.of(false,true)) for(boolean hat:List.of(false,true))
            cases.add(build("villager-"+(young?"young":"adult")+"-hat-"+hat,"minecraft:villager#main",young,-28f,hat,true,true,true));
        for(boolean young:List.of(false,true)) for(boolean posed:List.of(false,true))
            cases.add(build("stand-"+(young?"small":"adult")+"-"+(posed?"posed":"default"),"minecraft:armor_stand#main",young,53f,true,posed,posed,posed));
        cases.add(build("stand-small-posed-no-arms-no-base","minecraft:armor_stand#main",true,-37f,true,false,false,true));
        Files.writeString(Path.of(args[0]),new GsonBuilder().disableHtmlEscaping().create().toJson(Map.of("version","1.21.1","cases",cases)));
        System.out.println("NPC 模型参考病例："+cases.size());
    }
}
