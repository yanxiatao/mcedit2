import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;

/** 仅截获原生渲染器提交边界；保留保存载荷、绘制分支、状态变更和原生 PoseStack。 */
public final class PistonRenderAdapter implements Opcodes {
 public static void main(String[] args)throws Exception {
  var input=new ClassReader(Files.readAllBytes(Path.of(args[0])));
  boolean legacy=args[2].equals("1.12.2"),v21=args[2].equals("1.21.1");
  String position=legacy?"et":v21?"jd":"gu",state=legacy?"awt":v21?"dtc":"dcb",pose=v21?"fbi":"eij";
  String entity=legacy?"awm":v21?"dsy":"dbx",cache=v21?"gfv":"fkq";
  String helper=legacy?"(Let;Lawt;Lbuk;Lamu;Z)Z":v21?"(Ljd;Ldtc;Lfbi;Lgez;Ldcw;ZI)V":"(Lgu;Ldcb;Leij;Lfjx;Lcmm;ZI)V";
  var output=new ClassWriter(ClassWriter.COMPUTE_MAXS);int[] changed={0,0,0};
  input.accept(new ClassVisitor(ASM8,output){
   String owner,parent;
   @Override public void visit(int version,int access,String name,String signature,String superclass,String[] interfaces){owner=name;parent=superclass;super.visit(version,access,name,signature,superclass,interfaces);}
   @Override public MethodVisitor visitMethod(int access,String name,String desc,String signature,String[] exceptions){
    var method=super.visitMethod(access,name,desc,signature,exceptions);
    if(name.equals("<init>")){
     changed[0]++;method.visitCode();method.visitVarInsn(ALOAD,0);method.visitMethodInsn(INVOKESPECIAL,parent,"<init>","()V",false);method.visitInsn(RETURN);method.visitMaxs(1,8);method.visitEnd();return null;
    }
    if(name.equals("a")&&desc.equals(helper)){
     changed[1]++;method.visitCode();method.visitVarInsn(ALOAD,1);method.visitVarInsn(ALOAD,2);
     if(!legacy)method.visitVarInsn(ALOAD,3);
     method.visitVarInsn(ILOAD,legacy?5:6);
     String capture="(L"+position+";L"+state+";"+(legacy?"":"L"+pose+";")+"Z)"+(legacy?"Z":"V");
     method.visitMethodInsn(INVOKESTATIC,"PistonRenderReference","record",capture,false);method.visitInsn(legacy?IRETURN:RETURN);method.visitMaxs(4,8);method.visitEnd();return null;
    }
    if(!name.equals("a")||!desc.startsWith("(L"+entity+";"))return method;
    changed[2]++;
    return new MethodVisitor(ASM8,method){
     boolean levelCheck;
     void drop(int opcode,String descriptor){var types=Type.getArgumentTypes(descriptor);for(int i=types.length-1;i>=0;i--)super.visitInsn(types[i].getSize()==2?POP2:POP);if(opcode!=INVOKESTATIC)super.visitInsn(POP);}
     @Override public void visitJumpInsn(int opcode,Label label){
      if(!legacy&&!levelCheck&&opcode==IFNONNULL){levelCheck=true;super.visitInsn(POP);super.visitInsn(ICONST_1);super.visitJumpInsn(IFNE,label);return;}
      super.visitJumpInsn(opcode,label);
     }
     @Override public void visitFieldInsn(int opcode,String owner,String field,String d){
      if(legacy&&opcode==GETSTATIC&&(owner.equals("cdy")||owner.equals("cdp")||owner.startsWith("bus$"))){super.visitInsn(ACONST_NULL);return;}
      super.visitFieldInsn(opcode,owner,field,d);
     }
     @Override public void visitMethodInsn(int opcode,String cls,String method,String d,boolean itf){
      if(!legacy&&cls.equals(cache)&&(method.equals("a")||method.equals("b"))&&d.equals("()V")){return;}
      if(legacy){
       if(cls.equals("bus")||cls.equals("bhz")||(cls.equals(owner)&&d.equals("(Lnf;)V"))){drop(opcode,d);return;}
       if(cls.equals("bib")&&method.equals("y")&&d.equals("()Z")){super.visitInsn(ICONST_0);return;}
       if(cls.equals(owner)&&d.equals("()Lamu;")){super.visitInsn(POP);super.visitInsn(ACONST_NULL);return;}
       if(cls.equals("bve")){drop(opcode,d);if(!Type.getReturnType(d).equals(Type.VOID_TYPE))super.visitInsn(ACONST_NULL);return;}
       if(cls.equals("buk")&&method.equals("c")&&d.equals("(DDD)V")){super.visitMethodInsn(INVOKESTATIC,"PistonRenderReference","offset","(Lbuk;DDD)V",false);return;}
       if(cls.equals("buk")&&d.equals("(ILcea;)V")){drop(opcode,d);return;}
      }
      super.visitMethodInsn(opcode,cls,method,d,itf);
     }
    };
   }
  },0);
  if(changed[0]!=1||changed[1]!=1||changed[2]!=1)throw new IllegalStateException("原生渲染入口不唯一："+java.util.Arrays.toString(changed));
  Files.write(Path.of(args[1]),output.toByteArray());
 }
}
