import com.google.gson.GsonBuilder;
import java.nio.file.*;
import java.util.*;
import java.util.function.Consumer;
/** 公开模型层、ModelPart.visit / Cube.compile 与无世界物品姿态参照；不使用反射或图形窗口。 */
public final class ItemSpecialReference {
 static final class Sink implements fbm {
  final List<float[]> vertices=new ArrayList<>();
  public fbm a(float x,float y,float z){vertices.add(new float[]{x,y,z,0,0});return this;}
  public fbm a(int r,int g,int b,int a){return this;}
  public fbm a(float u,float v){var p=vertices.getLast();p[3]=u;p[4]=v;return this;}
  public fbm a(int u,int v){return this;}public fbm b(int u,int v){return this;}
  public fbm b(float x,float y,float z){return this;}
 }
 static final Map<String,List<float[]>> cases=new TreeMap<>();
 static fyk root(String name){return fyh.a().entrySet().stream().filter(e->(e.getKey().a()+"#"+e.getKey().b()).equals("minecraft:"+name)).findFirst().orElseThrow().getValue().a();}
 static void write(String name,String layer,Consumer<fbi> transform,Consumer<fyk> pose,Set<String> parts){
  fyk root=root(layer);pose.accept(root);fbi stack=new fbi();stack.a(-0.5f,-0.5f,-0.5f);transform.accept(stack);
  var output=new ArrayList<float[]>();
  root.a(stack,(matrix,path,index,cube)->{
   if(!parts.isEmpty()&&!parts.contains(path))return;
   var sink=new Sink();cube.a(matrix,sink,0,0,-1);output.addAll(sink.vertices);
  });cases.put(name,output);
 }
 public static void main(String[]args)throws Exception{
  ab.a();akt.a();
  write("chest","chest#main",s->{},r->{},Set.of());
  for(String part:List.of("head","foot"))write("bed_"+part,"bed_"+part+"#main",s->{s.a(0,0.5625f,part.equals("foot")?-1:0);s.a(a.b.rotationDegrees(90));s.a(0.5f,0.5f,0.5f);s.a(a.f.rotationDegrees(180));s.a(-0.5f,-0.5f,-0.5f);},r->{},Set.of());
  write("shulker","shulker#main",s->{s.a(0.5f,0.5f,0.5f);s.b(0.9995f,0.9995f,0.9995f);s.b(1,-1,-1);s.a(0,-1,0);},r->r.b("lid").a(0,24,0),Set.of("/base","/lid"));
  write("banner","banner#main",s->{s.a(0.5f,0.5f,0.5f);s.b(0.6666667f,-0.6666667f,-0.6666667f);},r->{var flag=r.b("flag");flag.c=-32;flag.e=(-0.0125f+0.01f*ayo.b(0.0f))*3.1415927f;},Set.of());
  write("shield","shield#main",s->s.b(1,-1,-1),r->{},Set.of());
  write("conduit","conduit#shell",s->s.a(0.5f,0.5f,0.5f),r->{},Set.of());
  write("pot_base","decorated_pot_base#main",s->{},r->{},Set.of());
  write("pot_sides","decorated_pot_sides#main",s->{},r->{},Set.of());
  for(String name:List.of("skeleton_skull","wither_skeleton_skull","zombie_head","creeper_head","player_head"))write(name,name+"#main",s->{s.a(0.5f,0,0.5f);s.b(-1,-1,1);},r->new fxg(r).a(0,180,0),Set.of());
  write("piglin_head","piglin_head#main",s->{s.a(0.5f,0,0.5f);s.b(-1,-1,1);},r->new fwn(r).a(0,180,0),Set.of());
  write("dragon_head","dragon_skull#main",s->{s.a(0.5f,0,0.5f);s.b(-1,-1,1);s.a(0,-0.374375f,0);s.b(0.75f,0.75f,0.75f);},r->new fye(r).a(0,180,0),Set.of());
  Files.writeString(Path.of(args[0]),new GsonBuilder().create().toJson(cases));
 }
}
