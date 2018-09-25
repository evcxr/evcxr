* If we get an error in generated code, tell the user a command to start
  investigation.
* Try using a workspace instead of setting target directory, copying Cargo.lock
  etc.
* Consider adding a crate to aid in interfacing with Evcxr.
* Compile item-only crates as rlibs instead of dylibs to avoid having them get
  recompiled next line.
* Tab completion. Perhaps bring up RLS and query it to determine completion options.
* Allow history of session to be written as a crate.
* Allow history of session to be written as a test.
* Allow a block of code to extend over multiple lines.
* Allow customization of colors.
* Allow some form of startup scripting - or at least a way to load the crate
  from the working dir.
* Automatically make all items pub
  * Probably not really practical while we can't make use of spans from syn.
* Investigate lack of warning: function cannot return without recurring
  * Probably we're not currently showing any warnings (if compilation
    succeeds). Perhaps we should.
* Consider emitting compilation errors as HTML and adding an "explain" link.
  
