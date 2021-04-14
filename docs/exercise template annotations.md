Exercise templates support several commented annotations that are processed when preparing exercise stubs (what students initially get when they start doing the exercise) and solutions. Note that the comment syntax is language dependent, `//` is used here as an example, but in Python an annotation would look like `# SOLUTION FILE`, for example.

### `// SOLUTION FILE`
Files that contain this annotation are left out of the stub. For example a file like
```Java
// SOLUTION FILE
public class Solution {
    public String solution() {
        return "solution";
    }
}
```
would look like this in the exercise solution
```Java
public class Solution {
    public String solution() {
        return "solution";
    }
}
```
but would be left out entirely from the stub.

### `// BEGIN SOLUTION`
### `// END SOLUTION`
Everything between these two annotations is left out of the stub. For example a file like
```Java
public class Solution {
    public String solution() {
        // BEGIN SOLUTION
        return "solution";
        // END SOLUTION
    }
}
```
would look like this in the exercise solution
```Java
public class Solution {
    public String solution() {
        return "solution";
    }
}
```
and like this in the exercise stub
```Java
public class Solution {
    public String solution() {
    }
}
```

### `// STUB:`
Code after this annotation is added to the stub but left out of the solution.  For example a file like
```Java
public class SomeClass {
    public String SomeFunction() {
        // STUB: return "stub";
        // BEGIN SOLUTION
        return "solution";
        // END SOLUTION
    }
}
```
would look like this in the exercise solution
```Java
public class SomeClass {
    public String SomeFunction() {
        return "solution";
    }
}
```
and like this in the exercise stub
```Java
public class SomeClass {
    public String SomeFunction() {
        return "stub";
    }
}
```
This example also shows use of the annotations together.

### `// HIDDEN FILE`
Files with this annotation are left out of the stub and solution entirely. This is useful for hidden test files that should be ran on the server, but not exposed to the students.

### `// BEGIN HIDDEN`
### `// END HIDDEN`
Code between these annotations is left out of the stub and solution entirely. This is useful for hidden tests that should be ran on the server, but not exposed to the students.
