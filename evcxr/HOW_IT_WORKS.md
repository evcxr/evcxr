# How it works

* We parse the supplied string using the syn crate in order to split the code
  into separate statements. How it does this is pretty gross since unfortunately
  the syn AST won't currently give us spans we can use, but hopefully that'll be
  resolved eventually. This is done in statement\_splitter.rs.

* We also use syn to identify the type of each statement, be it an item
  (function, struct, enum etc) or a statement or expression.
  
* If we've got a statement or expression, we wrap it in a generated function
  body with a unique name. Extra code added to the function is responsible for
  saving and restoring variables between executions, handling panics etc.
  
* We write the code as a crate then get cargo to build it and write the result
  as a shared object (e.g. a .so file on Linux).
  
* We open the shared object (e.g. using dlopen on Linux), look up the symbol for
  our function and call it.
  
* There's a small runtime crate (evcxr\_internal\_runtime) that gets added as a
  dependency. This holds all variables in a ```HashMap<String, Box<Any +
  static>>```.
  
* We look for variable declarations in the syntax tree we got from the syn crate
  and add code to store those variables into the map.
  
* Next time we run some code, we move the variable values back out of the map,
  restoring them with the same name and type as before.
  
* In order to restore variables with their correct type, we attempt to store
  them into the map as type String. When rustc gives us a compilation error, it
  tells us their actual type. We then compile again with the corrected types.

* Fortunately rustc can be asked to emit errors as JSON and we've recorded
  metadata about each line of source as we write it out, so this ends up less
  hacky than it sounds (although it's still obviously not ideal).

* We also use compilation errors to tell us:
  * Whether a variable has been moved, so is no longer available.
  * Whether a variable is non-copy and is referenced by the code being run. This
    allows us to restrict variables lost during a panic to only these variables.
  * Whether the final expression implements a method evcxr\_display.

* All user code is run in a subprocess with which we communicate via
  stdin/stdout, giving it some simple commands to do things like load a .so file
  and run a user function contained within.

* Using a subprocess has several advantages:
  * It allows us to restart everything if the subprocess segfaults due to some
    bad unsafe code.
  * It's probably easier to port since we don't need to capture our own
    stdout/stderr.
  * We can use out stdout/stderr for printing stuff, since we didn't redirect
    them.
  * It keeps things isolated if running multiple EvaluationContexts at once
    (e.g. from tests).
