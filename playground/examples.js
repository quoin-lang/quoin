// Playground example programs. Every snippet here is executed against the real wasm
// bundle by scripts/wasm_examples_check.mjs — run it after editing.
export const examples = {
  "Hello, Quoin": `'Hello from Quoin, running entirely in your browser!'.print

"* The final expression's value appears in the result line below.
(1..11).reduce:{ |sum n| sum + n }
`,

  "Blocks & collections": `"* Blocks are closures; collections iterate with them.
var squares = (1..10).collect:{ |n| n * n }
squares.print

"* Ranges are end-exclusive: 1..10 is 1 through 9.
squares.reduce:{ |sum n| sum + n }
`,

  "Classes": `Person <- { |@name @age|
    init: -> { |name age|
        @name = name
        @age = age
    }

    greet -> { 'Hi, I am ' + @name + ' (' + @age.s + ')' }
}

var ada = Person.new:{
    var name = 'Ada'
    var age = 36
}
ada.greet
`,

  "Fibonacci, timed": `Fib <- {
    .meta <-- {
        value: -> { |n: Integer ^Integer|
            (n <= 1).if:{ ^n } else:{ ^(.value:(n - 1)) + (.value:(n - 2)) }
        }
    }
}

var micros = Timer.time:{ (Fib.value:24).print };
%'computed in %{micros} microseconds'
`,

  "Errors are values": `"* catch: turns any raised error into a handled value.
var caught = { 10 / 0 }.catch:{ |e| 'caught: ' + e.s }
caught.print

"* OS things (sockets, files, processes, fibers) don't exist in the
"* browser sandbox -- they raise ordinary catchable errors too.
{ TcpSocket.connect:'example.com:80' }.catch:{ |e| e.s }
`,

  "JSON & maps": `var parsed = JSON.parse:'["smalltalk", "ruby", "quoin"]'
parsed.print

"* Maps keep insertion order; keys are quoted.
JSON.generate:#{ 'name': 'Quoin' 'inBrowser': true }
`,
};
