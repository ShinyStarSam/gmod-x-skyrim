// For each text given (script arguments after the output path), list the DLL's strings that
// contain it, the functions that use each string, and those functions decompiled. Read-only.
//   tools\ghidra_dump.ps1 -Script StringRefs -Out build\ghidra\refs.txt -Offsets ShouldCollide -Dll ...\server.dll
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.data.StringDataInstance;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

public class StringRefs extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        PrintWriter out = new PrintWriter(new FileWriter(args[0]));
        long base = currentProgram.getImageBase().getOffset();
        DecompInterface di = new DecompInterface();
        di.openProgram(currentProgram);
        Set<Function> done = new HashSet<>();
        for (int i = 1; i < args.length; i++) {
            out.printf("#################### strings containing \"%s\" ####################%n", args[i]);
            for (Data d : currentProgram.getListing().getDefinedData(true)) {
                if (!d.hasStringValue()) continue;
                String s = StringDataInstance.getStringDataInstance(d).getStringValue();
                if (s == null || !s.contains(args[i])) continue;
                out.printf("string +0x%x \"%s\"%n", d.getAddress().getOffset() - base, s.length() > 120 ? s.substring(0, 120) : s);
                for (Reference r : getReferencesTo(d.getAddress())) {
                    Function f = getFunctionContaining(r.getFromAddress());
                    out.printf("   used at +0x%x in %s%n", r.getFromAddress().getOffset() - base,
                        f == null ? "NO FUNCTION" : f.getName() + " @ +0x" + Long.toHexString(f.getEntryPoint().getOffset() - base));
                    if (f == null || done.contains(f) || done.size() > 40) continue;
                    done.add(f);
                    out.println("   -- callers:");
                    for (Reference c : getReferencesTo(f.getEntryPoint())) {
                        Function cf = getFunctionContaining(c.getFromAddress());
                        out.printf("      +0x%x %s%n", c.getFromAddress().getOffset() - base, cf == null ? "?" : cf.getName());
                    }
                    DecompileResults res = di.decompileFunction(f, 120, monitor);
                    out.println("   -- decompiled:");
                    out.println(res.decompileCompleted() ? res.getDecompiledFunction().getC() : "FAILED " + res.getErrorMessage());
                }
            }
        }
        out.close();
    }
}
