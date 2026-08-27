//! The files a genuinely empty project starts life with.
//!
//! Split out of the old `images` module, which generated a `Dockerfile` and
//! these together. The `Dockerfile` went with Docker; the starter files did
//! not, because a new project still needs something to run.
//!
//! Nothing here touches the disk — `lifecycle::scaffold` does that, and never
//! overwrites a file that already exists.

/// The "hello world" a genuinely empty project needs, as `(path, contents)`.
///
/// Only ever written when absent, so a fetched repository never receives any of
/// it — its own files are already there.
pub fn starter_files(runtime: &str) -> Vec<(&'static str, &'static str)> {
    match runtime {
        "NODEJS" => vec![
            ("index.js", "console.log('Hello from Panel Platform');\n"),
            (
                "package.json",
                "{\n  \"name\": \"project\",\n  \"private\": true,\n  \"version\": \"1.0.0\",\n  \"main\": \"index.js\"\n}\n",
            ),
        ],
        "TYPESCRIPT" => vec![
            (
                "src/index.ts",
                "console.log('Hello from Panel Platform');\n",
            ),
            (
                "package.json",
                "{\n  \"name\": \"project\",\n  \"private\": true,\n  \"version\": \"1.0.0\",\n  \"scripts\": {\n    \"build\": \"tsc\",\n    \"start\": \"node dist/index.js\"\n  }\n}\n",
            ),
            (
                "tsconfig.json",
                "{\n  \"compilerOptions\": {\n    \"target\": \"ES2022\",\n    \"module\": \"NodeNext\",\n    \"outDir\": \"dist\",\n    \"strict\": true\n  },\n  \"include\": [\"src\"]\n}\n",
            ),
        ],
        "BUN" => vec![
            ("index.ts", "console.log('Hello from Panel Platform');\n"),
            // `bunfig.toml` is what makes this a Bun project rather than a Node
            // one, and detection reads it back to decide the runtime.
            ("bunfig.toml", "# Bun configuration.\n"),
            (
                "package.json",
                "{\n  \"name\": \"project\",\n  \"private\": true,\n  \"version\": \"1.0.0\"\n}\n",
            ),
        ],
        "DENO" => vec![
            ("main.ts", "console.log('Hello from Panel Platform');\n"),
            ("deno.json", "{\n  \"tasks\": {}\n}\n"),
        ],
        "PYTHON" => vec![
            ("main.py", "print('Hello from Panel Platform', flush=True)\n"),
            ("requirements.txt", ""),
        ],
        "GO" => vec![
            (
                "main.go",
                "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"Hello from Panel Platform\")\n}\n",
            ),
            ("go.mod", "module project\n\ngo 1.23\n"),
        ],
        "RUST" => vec![
            (
                "src/main.rs",
                "fn main() {\n    println!(\"Hello from Panel Platform\");\n}\n",
            ),
            (
                "Cargo.toml",
                "[package]\nname = \"project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            ),
        ],
        "JAVA" => vec![
            (
                "src/main/java/Main.java",
                "public class Main {\n    public static void main(String[] args) {\n        System.out.println(\"Hello from Panel Platform\");\n    }\n}\n",
            ),
            // Without a build file this is a directory of Java source, and
            // detection would not call it a Java project at all.
            (
                "pom.xml",
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>platform.panel</groupId>\n  <artifactId>project</artifactId>\n  <version>1.0.0</version>\n  <properties>\n    <maven.compiler.release>21</maven.compiler.release>\n  </properties>\n</project>\n",
            ),
        ],
        "PHP" => vec![(
            "index.php",
            "<?php\n\necho \"Hello from Panel Platform\\n\";\n",
        )],
        "RUBY" => vec![
            ("app.rb", "puts 'Hello from Panel Platform'\n"),
            ("Gemfile", "source 'https://rubygems.org'\n"),
        ],
        "DOTNET" => vec![
            (
                "Program.cs",
                "System.Console.WriteLine(\"Hello from Panel Platform\");\n",
            ),
            // The project file is the marker, and its stem becomes the assembly
            // name the start command expects.
            (
                "app.csproj",
                "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>Exe</OutputType>\n    <TargetFramework>net8.0</TargetFramework>\n  </PropertyGroup>\n</Project>\n",
            ),
        ],
        "STATIC" => vec![(
            "public/index.html",
            "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <title>Panel Platform</title>\n  </head>\n  <body>\n    <h1>It works</h1>\n  </body>\n</html>\n",
        )],
        // Nothing sensible to scaffold: a polyglot project is by definition one
        // that already has files in more than one language.
        _ => Vec::new(),
    }
}
