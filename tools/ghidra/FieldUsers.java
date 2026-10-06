// List every instruction whose memory operand uses one of the given displacements (e.g. an
// entity field offset such as 0x14ac), grouped by function. Read-only.
//   tools\ghidra_dump.ps1 -Script FieldUsers -Out build/ghidra/fields.txt -Offsets 0x14ac -Dll <path to server.dll>
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import ghidra.program.model.scalar.Scalar;
import java.io.*;
import java.util.*;

public class FieldUsers extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        PrintWriter out = new PrintWriter(new FileWriter(args[0]));
        long base = currentProgram.getImageBase().getOffset();
        Set<Long> wanted = new HashSet<>();
        for (int i = 1; i < args.length; i++) wanted.add(Long.decode(args[i]));
        Map<String, List<String>> byFunction = new TreeMap<>();
        for (Instruction ins : currentProgram.getListing().getInstructions(true)) {
            for (int op = 0; op < ins.getNumOperands(); op++) {
                String rep = ins.getDefaultOperandRepresentation(op);
                if (!rep.contains("[")) continue;
                boolean hit = false;
                for (Object o : ins.getOpObjects(op)) {
                    if (o instanceof Scalar && wanted.contains(((Scalar) o).getUnsignedValue())) hit = true;
                }
                if (!hit) continue;
                Function f = getFunctionContaining(ins.getAddress());
                String key = f == null ? "NO FUNCTION" : String.format("%s @ +0x%x", f.getName(), f.getEntryPoint().getOffset() - base);
                byFunction.computeIfAbsent(key, k -> new ArrayList<>())
                    .add(String.format("   +0x%x  %s", ins.getAddress().getOffset() - base, ins));
            }
        }
        for (Map.Entry<String, List<String>> e : byFunction.entrySet()) {
            out.println(e.getKey());
            for (String s : e.getValue()) out.println(s);
        }
        out.close();
    }
}
