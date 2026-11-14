import com.google.gson.GsonBuilder;
import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;
import java.util.concurrent.atomic.AtomicInteger;

/** 官方注册 SCALE 属性的公开实例、运算与 NBT 持久化参照。 */
public final class ScaleAttributeReference {
    private static buu modifier(String id,double amount,String operation) {
        var tag=new ub();tag.a("id","oracle:"+id);tag.a("amount",amount);tag.a("operation",operation);
        return Objects.requireNonNull(buu.a(tag),"官方修饰符 NBT 解码失败");
    }
    private static Map<String,Object> snapshot(String id,bus instance,AtomicInteger events) throws Exception {
        double value=instance.f();var saved=instance.g();
        var bytes=new ByteArrayOutputStream();
        try(var data=new DataOutputStream(bytes)) { uo.a(saved,(java.io.DataOutput)data); }
        return Map.of("id",id,"value",value,"base",instance.b(),"dirty_events",events.get(),
            "nbt_base64",Base64.getEncoder().encodeToString(bytes.toByteArray()),"nbt_snbt",saved.toString());
    }
    private static bus create(AtomicInteger events) { return new bus(buw.y,ignored -> events.incrementAndGet()); }
    public static void main(String[] args) throws Exception {
        ab.a();akt.a();var cases=new ArrayList<Object>();
        var events=new AtomicInteger();var inst=create(events);cases.add(snapshot("default",inst,events));
        events=new AtomicInteger();inst=create(events);inst.a(2.0);
        inst.d(modifier("add",1.0,"add_value"));inst.d(modifier("base",0.5,"add_multiplied_base"));inst.d(modifier("total",0.25,"add_multiplied_total"));
        cases.add(snapshot("combined",inst,events));
        events=new AtomicInteger();inst=create(events);inst.a(-5.0);cases.add(snapshot("below_min",inst,events));
        events=new AtomicInteger();inst=create(events);inst.a(100.0);cases.add(snapshot("above_max",inst,events));
        events=new AtomicInteger();inst=create(events);inst.d(modifier("duplicate",0.5,"add_value"));
        String duplicate;
        try { inst.d(modifier("duplicate",0.8,"add_multiplied_total")); throw new IllegalStateException("重复 ID 未抛异常"); }
        catch(IllegalArgumentException expected) { duplicate=expected.getClass().getName()+": "+expected.getMessage(); }
        cases.add(snapshot("duplicate_rejected",inst,events));
        inst.c(modifier("duplicate",0.8,"add_multiplied_total"));cases.add(snapshot("duplicate_replaced",inst,events));
        Files.writeString(Path.of(args[0]),new GsonBuilder().disableHtmlEscaping().create().toJson(
            Map.of("version","1.21.1","cases",cases,"duplicate_add_exception",duplicate)));
        System.out.println("SCALE 属性参考病例："+cases.size());
    }
}
