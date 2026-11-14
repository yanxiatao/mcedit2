import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/** 直接调用官方 1.21.1 的坐标种子、LegacyRandomSource、WeightedRandom 及原版方块状态。
 * 不使用反射，不启动客户端窗口，不读取或写入世界；固定混淆名来自官方映射。
 */
public final class ModelRandomReference {
    private static final int[][] WEIGHTS = {{1,3},{1,1},{2,3,5},{1073741823,1073741823,1},{1}, new int[96]};
    static { java.util.Arrays.fill(WEIGHTS[5], 1); }
    private static final int[][] EDGES = {{0,0,0},{1,64,1},{-1,-64,-1},{29999999,319,-29999999},
        {-29999999,4000,29999999},{Integer.MIN_VALUE,Integer.MAX_VALUE,Integer.MAX_VALUE},
        {Integer.MAX_VALUE,Integer.MIN_VALUE,Integer.MIN_VALUE}};

    private static String quote(String value) {
        return "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
    }
    private static String xyz(int[] p) { return "["+p[0]+","+p[1]+","+p[2]+"]"; }

    /** 返回真正 WeightedRandom 区间查询结果，包括负数下标的客户端语义。 */
    private static int selected(long seed, int[] weights) {
        var entries = new ArrayList<bpd.b<Integer>>();
        for (int i=0;i<weights.length;i++) entries.add(bpd.a(i,weights[i]));
        if (entries.size()==1) return entries.getFirst().b();
        int index = Math.abs((int)ayw.a(seed).g()) % bpe.a(entries);
        return bpe.a(entries,index).orElseThrow().b();
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static String propertyValue(duf property, Comparable value) { return property.a(value); }

    public static void main(String[] args) throws Exception {
        var positions = new ArrayList<int[]>();
        for (int[] edge: EDGES) positions.add(edge);
        var random = ayw.a(4189567L);
        for (int i=0;i<256;i++) positions.add(new int[]{random.f(),random.f(),random.f()});
        var out = new StringBuilder("{\"version\":\"1.21.1\",\"random\":[");
        boolean comma = false;
        for (int[] pos: positions) {
            long seed = ayo.b(pos[0],pos[1],pos[2]);
            long next = ayw.a(seed).g();
            for (int[] weights: WEIGHTS) {
                if (comma) out.append(','); comma=true;
                out.append("{\"pos\":").append(xyz(pos)).append(",\"seed\":").append(seed)
                    .append(",\"next_long\":").append(next).append(",\"weights\":[");
                for (int i=0;i<weights.length;i++) { if(i>0) out.append(','); out.append(weights[i]); }
                out.append("],\"direct\":").append(selected(seed,weights))
                    .append(",\"multipart\":").append(selected(next,weights)).append('}');
            }
        }
        // 独立覆盖 nextLong 低位为 MIN_VALUE 时的 Math.abs 溢出行为。
        long inverse = java.math.BigInteger.valueOf(25214903917L).modInverse(java.math.BigInteger.ONE.shiftLeft(48)).longValueExact();
        long boundaryState = 0x800000000000L;
        for(int i=0;i<2;i++) boundaryState=(boundaryState-11)*inverse & ((1L<<48)-1);
        long specialSeed = boundaryState ^ 25214903917L;
        if ((int)ayw.a(specialSeed).g()!=Integer.MIN_VALUE) throw new IllegalStateException("随机边界参照构造失败");
        out.append("],\"special\":["); comma=false;
        for(long seed: new long[]{Long.MIN_VALUE,Long.MAX_VALUE,0,-1,specialSeed}) {
            if(comma) out.append(','); comma=true;
            out.append("{\"seed\":").append(seed).append(",\"next_long\":").append(ayw.a(seed).g())
                .append(",\"choice\":").append(selected(seed,new int[]{2,3,5})).append('}');
        }
        // 初始化内建注册表以查询实际继承关系和所有相关状态，不构造世界。
        ab.a(); akt.a();
        out.append("],\"seed_rules\":{"); comma=false;
        var blocks = new ArrayList<dfy>();
        for (dfy block: lt.e) {
            String rule = block instanceof dfr ? "bed" : block instanceof die || block instanceof dig ? "lower" : null;
            if (rule == null) continue;
            blocks.add(block);
            if(comma) out.append(','); comma=true;
            out.append(quote(lt.e.b(block).toString())).append(':').append(quote(rule));
        }
        out.append("},\"block_seeds\":["); comma=false;
        for (dfy block: blocks) for (dtc state: block.l().a()) {
            for (int k=0;k<4;k++) {
                int[] pos=EDGES[k];
                if(comma) out.append(','); comma=true;
                out.append("{\"name\":").append(quote(lt.e.b(block).toString())).append(",\"properties\":{");
                boolean propertyComma=false;
                for (Map.Entry<duf<?>,Comparable<?>> entry: state.C().entrySet()) {
                    if(propertyComma) out.append(','); propertyComma=true;
                    out.append(quote(entry.getKey().f())).append(':')
                        .append(quote(propertyValue(entry.getKey(),entry.getValue())));
                }
                out.append("},\"pos\":").append(xyz(pos)).append(",\"seed\":")
                    .append(state.a(new jd(pos[0],pos[1],pos[2]))).append('}');
            }
        }
        out.append("]}");
        Files.writeString(Path.of(args[0]),out.toString(),StandardCharsets.UTF_8);
    }
}
