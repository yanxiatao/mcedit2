import com.google.gson.Gson;
import java.nio.*;
import java.nio.file.*;
import java.util.*;
import org.lwjgl.util.vector.Matrix4f;
import org.lwjgl.util.vector.Vector3f;
import org.lwjgl.util.vector.Vector4f;

/** 1.12.2 原生 ModelBox / TexturedQuad -> CPU 提交。只读取公开的部件与缓冲 API。 */
public final class LegacyEntityReference {
    static List<float[]> active;
    /** 取代临时分析副本中唯一 GPU 提交方法；消费真实四角缓冲并完成重置。 */
    public static void capture(buk buffer) {
        if(active==null||buffer.h()!=4||buffer.i()!=7||buffer.g().b(0)!=12||buffer.g().g()!=24)
            throw new IllegalStateException("不是预期的四角 POSITION_TEX_NORMAL 提交");
        ByteBuffer bytes=buffer.f().duplicate().order(ByteOrder.nativeOrder());
        for(int vertex=0;vertex<4;vertex++) {
            int at=vertex*24;
            active.add(new float[]{bytes.getFloat(at),bytes.getFloat(at+4),bytes.getFloat(at+8),bytes.getFloat(at+12),bytes.getFloat(at+16)});
        }
        buffer.b();
    }
    static final Map<String,Object> layers=new TreeMap<>();
    static final Map<String,Object> metadata=new TreeMap<>();
    static Map<String,Object> reference;
    static final Map<String,Object> references=new TreeMap<>();
    static Map<String,Object> node(brs part,Map<brs,String> names,Map<String,Object> parts,String path,Matrix4f parent,boolean visible) {
        var cubes=new TreeMap<Integer,List<float[]>>();
        for(int i=0;i<part.l.size();i++) {
            active=new ArrayList<>();part.l.get(i).a(bve.a().c(),1f/16f);
            cubes.put(i,active);active=null;
        }
        var record=new TreeMap<String,Object>();
        record.put("pivot",new float[]{part.c+part.o*16,part.d+part.p*16,part.e+part.q*16});
        record.put("rotation",new float[]{part.f,part.g,part.h});record.put("scale",new float[]{1,1,1});
        record.put("visible",part.j&&!part.k);record.put("skip_draw",false);record.put("cubes",cubes);parts.put(path,record);
        var matrix=new Matrix4f(parent);
        Matrix4f.translate(new Vector3f(part.o+part.c/16f,part.p+part.d/16f,part.q+part.e/16f),matrix,matrix);
        Matrix4f.rotate(part.h,new Vector3f(0,0,1),matrix,matrix);
        Matrix4f.rotate(part.g,new Vector3f(0,1,0),matrix,matrix);
        Matrix4f.rotate(part.f,new Vector3f(1,0,0),matrix,matrix);
        visible=visible&&part.j&&!part.k;
        if(visible&&!cubes.isEmpty()) {
            var baked=new TreeMap<Integer,List<float[]>>();
            for(var entry:cubes.entrySet()){
                var vertices=new ArrayList<float[]>();
                for(var vertex:entry.getValue()){
                    var p=Matrix4f.transform(matrix,new Vector4f(vertex[0],vertex[1],vertex[2],1),null);
                    vertices.add(new float[]{p.x,p.y,p.z,vertex[3],vertex[4]});
                }
                baked.put(entry.getKey(),vertices);
            }
            reference.put(path,baked);
        }
        if(part.m!=null)for(int i=0;i<part.m.size();i++) {
            var child=part.m.get(i);String label=names.get(child);
            if(label==null)throw new IllegalStateException("未命名的原生子部件："+path+"/"+i);
            node(child,names,parts,path+"/"+label,matrix,visible);
        }
        return record;
    }
    static void layer(String key,bqf model,Object... entries) {
        var names=new IdentityHashMap<brs,String>();var roots=new ArrayList<brs>();
        for(int i=0;i<entries.length;i+=2){var part=(brs)entries[i+1];names.put(part,(String)entries[i]);roots.add(part);}
        for(var part:new ArrayList<>(roots))if(part.m!=null)for(var child:part.m)roots.remove(child);
        var parts=new TreeMap<String,Object>();
        reference=new TreeMap<String,Object>();
        parts.put("",Map.of("pivot",new float[]{0,0,0},"rotation",new float[]{0,0,0},"scale",new float[]{1,1,1},"visible",true,"skip_draw",false,"cubes",Map.of()));
        for(var part:roots)node(part,names,parts,"/"+names.get(part),new Matrix4f(),true);
        references.put(key,reference);
        layers.put(key,parts);metadata.put(key,Map.of("texture_width",model.s,"texture_height",model.t));
    }
    static void biped(String key,bpx model,Object... extra) {
        var entries=new ArrayList<Object>(List.of("head",model.e,"hat",model.f,"body",model.g,"right_arm",model.h,"left_arm",model.i,"right_leg",model.j,"left_leg",model.k));
        entries.addAll(List.of(extra));layer(key,model,entries.toArray());
    }
    /** 使用 ModelBase 公开注册列表；索引顺序来自准确构造器，不访问私有字段或反射。 */
    static void indexed(String key,bqf model,String... names) {
        var entries=new ArrayList<Object>();
        for(int index=0;index<names.length;index++){entries.add(names[index]);entries.add(model.r.get(index));}
        layer(key,model,entries.toArray());
    }
    static void guardian(String key) {
        // bpu 先注册 head、十二根 spike，再注册 eye 和三节 tail；父链来自原生构造器。
        var names=new ArrayList<String>();names.add("head");
        for(int i=0;i<12;i++)names.add("spike"+i);
        names.addAll(List.of("eye","tail0","tail1","tail2"));
        indexed(key,new bpu(),names.toArray(String[]::new));
    }
    static void quadruped(String key,bqm model) {
        model.a(0,0,0,0,0,1f/16f,null);
        layer(key,model,"head",model.a,"body",model.b,"right_hind_leg",model.c,"left_hind_leg",model.d,"right_front_leg",model.e,"left_front_leg",model.f);
    }
    static final class Cow extends bpn {float[] young(){return new float[]{g,h};}}
    static final class Pig extends bqi {Pig(float inflate){super(inflate);}float[] young(){return new float[]{g,h};}}
    static void llama(String key,float inflate) {
        var model=new bqc(inflate);
        model.a(0,0,0,0,0,1f/16f,null);
        // bqm 先注册六个部件，bqc 随后依次注册头、身体、两个箱子和四腿。
        layer(key,model,"head",model.a,"body",model.b,"right_hind_leg",model.c,"left_hind_leg",model.d,
            "right_front_leg",model.e,"left_front_leg",model.f,"right_chest",model.r.get(8),"left_chest",model.r.get(9));
    }
    static void horse(String key) {
        var model=new bpv();
        String[] names={"body","tail_base","tail_middle","tail_tip","left_hind_leg","left_hind_shin","left_hind_hoof",
            "right_hind_leg","right_hind_shin","right_hind_hoof","left_front_leg","left_front_shin","left_front_hoof",
            "right_front_leg","right_front_shin","right_front_hoof","head","upper_mouth","lower_mouth",
            "horse_left_ear","horse_right_ear","donkey_left_ear","donkey_right_ear","neck","left_chest","right_chest",
            "saddle_bottom","saddle_front","saddle_back","left_stirrup","left_stirrup_leather","right_stirrup","right_stirrup_leather",
            "left_saddle_mouth","right_saddle_mouth","left_saddle_line","right_saddle_line","mane","head_saddle"};
        if(model.r.size()!=names.length)throw new IllegalStateException("旧马原生构造部件数量发生变化");
        // 使用公开 boxList，不读取私有字段；静态目标沿 bpv.setLivingAnimations 的中立值。
        var upper=model.r.get(17);upper.a(0,.02f,.02f);upper.f=upper.g=upper.h=0;
        var lower=model.r.get(18);lower.a(0,0,0);lower.f=lower.g=lower.h=0;
        for(int index:new int[]{29,30,31,32})model.r.get(index).d=2;
        model.r.get(1).f=model.r.get(2).f=-1.3089969f;model.r.get(3).f=-1.5707963f;
        var entries=new ArrayList<Object>();
        for(int index=0;index<names.length;index++){entries.add(names[index]);entries.add(model.r.get(index));}
        layer(key,model,entries.toArray());
    }
    public static void main(String[] args)throws Exception {
        ni.c();
        biped("minecraft:zombie#main",new brl());biped("minecraft:zombie_pigman#main",new brl());biped("minecraft:skeleton#main",new bqw());
        biped("minecraft:stray#outer",new bqw(.25f,true));
        indexed("minecraft:slime#main",new bqy(16),"cube","right_eye","left_eye","mouth");
        indexed("minecraft:slime#outer",new bqy(0),"cube");
        indexed("minecraft:magma_cube#main",new bqa(),"cube0","cube1","cube2","cube3","cube4","cube5","cube6","cube7","inside_cube");
        indexed("minecraft:ghast#main",new bpt(),"body","tentacle0","tentacle1","tentacle2","tentacle3","tentacle4","tentacle5","tentacle6","tentacle7","tentacle8");
        indexed("minecraft:blaze#main",new bpi(),"part0","part1","part2","part3","part4","part5","part6","part7","part8","part9","part10","part11","head");
        indexed("minecraft:silverfish#main",new bqu(),"segment0","segment1","segment2","segment3","segment4","segment5","segment6","layer0","layer1","layer2");
        indexed("minecraft:endermite#main",new bpr(),"segment0","segment1","segment2","segment3");
        biped("minecraft:enderman#main",new bpq(0));
        biped("minecraft:zombie_villager#main",new brh());
        biped("minecraft:zombie_villager#inner_armor",new brh(.5f,0,true));
        biped("minecraft:zombie_villager#outer_armor",new brh(1,0,true));
        indexed("minecraft:illager#main",new bpy(0,0,64,64),"head","hat","nose","body","arms","left_shoulder","right_leg","left_leg","right_arm","left_arm");
        indexed("minecraft:witch#main",new bri(0),"head","nose","body","arms","right_leg","left_leg","mole","hat","hat2","hat3","hat4");
        var vex=new bre();biped("minecraft:vex#main",vex,"right_wing",vex.r.get(8),"left_wing",vex.r.get(9));
        var player=new bqj(0,false);biped("minecraft:player#main",player,"left_pants",player.a,"right_pants",player.b,"left_sleeve",player.c,"right_sleeve",player.d,"jacket",player.u);
        player=new bqj(0,true);biped("minecraft:player_slim#main",player,"left_pants",player.a,"right_pants",player.b,"left_sleeve",player.c,"right_sleeve",player.d,"jacket",player.u);
        var armor=new bpe();biped("minecraft:armor_stand#main",armor,"right_body_stick",armor.a,"left_body_stick",armor.b,"shoulder_stick",armor.c,"base_plate",armor.d);
        var cow=new Cow();quadruped("minecraft:cow#main",cow);var pig=new Pig(0);quadruped("minecraft:pig#main",pig);quadruped("minecraft:pig#saddle",new Pig(.5f));
        indexed("minecraft:chicken#main",new bpm(),"head","beak","red_thing","body","right_leg","left_leg","right_wing","left_wing");
        indexed("minecraft:rabbit#main",new bqn(),"left_hind_foot","right_hind_foot","left_haunch","right_haunch","body","left_front_leg","right_front_leg","head","right_ear","left_ear","tail","nose");
        var polar=new bqk();layer("minecraft:polar_bear#main",polar,"head",polar.a,"body",polar.b,"right_hind_leg",polar.c,"left_hind_leg",polar.d,"right_front_leg",polar.e,"left_front_leg",polar.f);
        indexed("minecraft:iron_golem#main",new brf(),"head","body","right_arm","left_arm","right_leg","left_leg");
        indexed("minecraft:snow_golem#main",new bqz(),"head","upper_body","lower_body","left_arm","right_arm");
        indexed("minecraft:ocelot#main",new bqg(),"head","body","tail1","tail2","right_hind_leg","left_hind_leg","right_front_leg","left_front_leg");
        indexed("minecraft:bat#main",new bpg(),"head","right_ear","left_ear","body","right_wing","right_wing_tip","left_wing","left_wing_tip");
        indexed("minecraft:squid#main",new brb(),"body","tentacle0","tentacle1","tentacle2","tentacle3","tentacle4","tentacle5","tentacle6","tentacle7");
        indexed("minecraft:parrot#main",new bqh(),"body","tail","left_wing","right_wing","head","head2","beak1","beak2","feather","left_leg","right_leg");
        guardian("minecraft:guardian#main");guardian("minecraft:elder_guardian#main");
        indexed("minecraft:wither#main",new brj(0),"shoulders","ribcage","tail","center_head","right_head","left_head");
        indexed("minecraft:wither#armor",new brj(.5f),"shoulders","ribcage","tail","center_head","right_head","left_head");
        var boat=new bpj();layer("minecraft:boat#main",boat,"hull_0",boat.a[0],"hull_1",boat.a[1],"hull_2",boat.a[2],"hull_3",boat.a[3],"hull_4",boat.a[4],"left_paddle",boat.b[0],"right_paddle",boat.b[1],"water_patch",boat.c);
        var cart=new bqe();layer("minecraft:minecart#main",cart,"bottom",cart.a[0],"side_1",cart.a[1],"side_2",cart.a[2],"side_3",cart.a[3],"side_4",cart.a[4],"inside",cart.a[5]);
        indexed("minecraft:shulker_bullet#main",new bqr(),"main");
        indexed("minecraft:llama_spit#main",new bqd(),"main");
        indexed("minecraft:evoker_fangs#main",new bps(),"base","upper_jaw","lower_jaw");
        indexed("minecraft:leash_knot#main",new bqb(),"knot");
        indexed("minecraft:end_crystal#main",new bro(0,true),"glass","cube","base");
        indexed("minecraft:wither_skull#main",new bqv(),"head");
        indexed("minecraft:ender_dragon#main",new brn(0),"head","jaw","neck","body","left_wing","left_wing_tip","left_front_leg","left_front_leg_tip","left_front_foot","left_hind_leg","left_hind_leg_tip","left_hind_foot");
        quadruped("minecraft:sheep#main",new bqp());quadruped("minecraft:sheep#fur",new bqo());
        var creeper=new bpo();layer("minecraft:creeper#main",creeper,"head",creeper.a,"body",creeper.c,"right_hind_leg",creeper.d,"left_hind_leg",creeper.e,"right_front_leg",creeper.f,"left_front_leg",creeper.g);
        var spider=new bra();layer("minecraft:spider#main",spider,"head",spider.a,"body0",spider.b,"body1",spider.c,"right_hind_leg",spider.d,"left_hind_leg",spider.e,"right_middle_hind_leg",spider.f,"left_middle_hind_leg",spider.g,"right_middle_front_leg",spider.h,"left_middle_front_leg",spider.i,"right_front_leg",spider.j,"left_front_leg",spider.k);
        var villager=new brg(0);layer("minecraft:villager#main",villager,"head",villager.a,"body",villager.b,"arms",villager.c,"right_leg",villager.d,"left_leg",villager.e,"nose",villager.f);
        var shulker=new bqs();layer("minecraft:shulker#main",shulker,"lid",shulker.b,"base",shulker.a,"head",shulker.c);
        // bpp 公开 boxList 保留构造顺序；中立 pivot / rotation 来自其 setupAnim。
        var elytra=new bpp();var leftWing=elytra.r.get(0);var rightWing=elytra.r.get(1);
        leftWing.c=5;rightWing.c=-5;leftWing.f=rightWing.f=.2617994f;
        leftWing.h=-.2617994f;rightWing.h=.2617994f;
        layer("minecraft:elytra#main",elytra,"left_wing",leftWing,"right_wing",rightWing);
        var wolf=new brk();
        var wolfHead=new brs(wolf,0,0);wolfHead.a(wolf.a.c,wolf.a.d,wolf.a.e);
        wolf.a.a(0,0,0);wolfHead.a(wolf.a);
        var wolfTail=new brs(wolf,0,0);wolfTail.a(wolf.g.c,wolf.g.d,wolf.g.e);wolfTail.f=.62831855f;
        wolf.g.a(0,0,0);wolfTail.a(wolf.g);
        wolf.b.f=wolf.h.f=1.5707964f;wolf.h.e=-3;
        layer("minecraft:wolf#main",wolf,"head",wolfHead,"real_head",wolf.a,"body",wolf.b,"upper_body",wolf.h,
            "right_hind_leg",wolf.c,"left_hind_leg",wolf.d,"right_front_leg",wolf.e,"left_front_leg",wolf.f,"tail",wolfTail,"real_tail",wolf.g);
        llama("minecraft:llama#main",0);llama("minecraft:llama#decor",.5f);
        for(String name:new String[]{"horse","donkey","mule","zombie_horse","skeleton_horse"})horse("minecraft:"+name+"#main");
        var sheepColors=new ArrayList<float[]>();for(var dye:ahs.values())sheepColors.add(aag.a(dye));
        Files.writeString(Path.of(args[0]),new Gson().toJson(Map.of("version","1.12.2","layers",layers,"reference",references,"metadata",metadata,"legacy",Map.of("young_cow",cow.young(),"young_pig",pig.young(),"sheep_colors",sheepColors))));
        System.out.println("原生旧模型层："+layers.size());
    }
}
