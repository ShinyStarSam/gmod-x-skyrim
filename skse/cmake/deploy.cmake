# Copies the built plugin into Skyrim's Data/SKSE/Plugins. A running Skyrim holds the DLL, so a
# failed copy is only a warning: the build itself succeeded.
foreach(file IN ITEMS "${DLL}" "${PDB}")
	execute_process(COMMAND "${CMAKE_COMMAND}" -E copy_if_different "${file}" "${DEST}/" RESULT_VARIABLE result)
	if(result EQUAL 0)
		message(STATUS "gmsky: copied ${file} to ${DEST}")
	else()
		message(WARNING "gmsky: couldn't copy ${file} to ${DEST} (is Skyrim running?)")
	endif()
endforeach()
