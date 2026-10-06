// Print the full disassembly of the functions at the given offsets. Read-only.
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import java.io.*;
public class Disasm extends GhidraScript {
    public void run() throws Exception {
        String[] a = getScriptArgs();
        PrintWriter out = new PrintWriter(new FileWriter(a[0]));
        long base = currentProgram.getImageBase().getOffset();
        for (int i = 1; i < a.length; i++) {
            Function f = getFunctionContaining(toAddr(base + Long.decode(a[i])));
            out.println("==== " + f.getName());
            for (Instruction ins : currentProgram.getListing().getInstructions(f.getBody(), true))
                out.printf("+0x%x  %s%n", ins.getAddress().getOffset() - base, ins);
        }
        out.close();
    }
}
