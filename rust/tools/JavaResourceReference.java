// 调用准确 1.21.1 客户端的公开资源接口，记录读取边界和动画元数据；不使用反射、不启动游戏。
import com.google.gson.*;
import java.io.StringReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.List;

public final class JavaResourceReference {
    public static void main(String[] args) throws Exception {
        JsonObject output=new JsonObject();output.addProperty("version","1.21.1");
        JsonArray documents=new JsonArray();
        for(String text:List.of("{\"x\":1}","{\"x\":1}r","{} {}","{} /* unterminated","// head\n{a:'value',}","{a:1}","{\"x\":}","[]","null","{\"animation\":{\"frames\":[0,1]}}")) {
            JsonObject row=new JsonObject();row.addProperty("input",text);
            try {row.add("value",aye.a(new StringReader(text)));row.addProperty("accepted",true);}
            catch(RuntimeException error) {row.addProperty("accepted",false);row.addProperty("error",error.toString());}
            documents.add(row);
        }
        output.add("documents",documents);
        JsonArray numbers=new JsonArray();
        for(String text:List.of("1.9","-0.9","-1.9","2147483648","4294967297","-4294967297","1e32","429496729700e-2","1e-2147483648","1e2147483647","1e2147483648","\"1\"","true","null")) {
            JsonObject row=new JsonObject();row.addProperty("input",text);
            try {row.addProperty("value",aye.g(JsonParser.parseString(text),"reference"));row.addProperty("accepted",true);}
            catch(RuntimeException error) {row.addProperty("accepted",false);row.addProperty("error",error.toString());}
            numbers.add(row);
        }
        output.add("numbers",numbers);
        JsonArray animations=new JsonArray();
        for(String text:List.of("{}","{\"frames\":[1]}","{\"frames\":[-1,0,1]}","{\"frames\":[null,[],0.9,{\"index\":1.9}]} क", "{\"width\":-1,\"height\":1.9}","{\"width\":4294967297,\"frames\":[4294967297]}","{\"frames\":[{\"index\":-1}]}","{\"frames\":[{\"index\":0,\"time\":0}]}","{\"frametime\":0.9}","{\"interpolate\":[]}","{\"interpolate\":\"true\"}","{\"interpolate\":\"TRUE\"}","{\"interpolate\":1}","{\"frametime\":7,\"frames\":[1,{\"index\":0,\"time\":3}]}","{\"frametime\":2147483647,\"frames\":[1,0]}","{\"frametime\":2.9,\"frames\":[-1,1,0]}")) {
            JsonObject row=new JsonObject();row.addProperty("input",text);
            try {
                grw metadata=new grx().b(aye.a(new StringReader(text)));
                gry size=metadata.a(4,8);
                row.addProperty("width",size.a());row.addProperty("height",size.b());
                row.addProperty("frametime",metadata.a());row.addProperty("interpolate",metadata.b());
                JsonArray frames=new JsonArray();metadata.a((index,time)->{JsonArray frame=new JsonArray();frame.add(index);frame.add(time);frames.add(frame);});
                row.add("frames",frames);row.addProperty("accepted",true);
            } catch(RuntimeException error) {row.addProperty("accepted",false);row.addProperty("error",error.toString());}
            animations.add(row);
        }
        output.add("animations",animations);
        Files.writeString(Path.of(args[0]),new GsonBuilder().setPrettyPrinting().create().toJson(output)+"\n",StandardCharsets.UTF_8);
    }
}
