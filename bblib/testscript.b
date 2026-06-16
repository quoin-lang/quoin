Point <- { | @x @y |
  .meta <-- {
    newX:y: -> { |x y|
      .new: { x = x; y = y }
    }
  }

  x -> { @x }
  y -> { @y }
  name -> { 'Point' }

  dist: -> { |other|
    dx = @x - other.x;
    dy = @y - other.y;
    ((dx * dx) + (dy * dy)).sqrt
  }
};

Point <- Point3D <- { | @z |
    z -> { @z }
};

PType <- {
    .mix:Point;
};

.print:'PType.name =' and:PType.new.name;

p1 = Point.newX: 3 y: 4;
p2 = Point.newX: 0 y: 0;
.print: 'p1.x =' and: p1.x;
.print: 'p1.y =' and: p1.y;
d = p1.dist: p2;
.print: 'distance =' and: d;
p1.print;
.print: 'p1.id =' and: p1.id;
.print: 'p2.id =' and: p2.id;
.print: 'p1.id =' and: p1.id;
.print: 'p2.id =' and: p2.id;

p3 = Point3D.new: { |x y z| x = 10; y = 20; z = 30 };
p3.print;
.print:p3.class;
.print:p3.class.s;
.print:p3.class.name;
.print:p3.class.class;
.print:p3.class.class.name;
.print:p3.class.parent;
.print:p3.class.parent.name;
.print:p3.class.parent.parent;
.print:p3.class.parent.parent.name;
.print:(p3.id==p3.id);
.print:(p3.id!=p1.id);
.print:(p3.class==p3.class);
.print:(p3.class!=p1.class);
p3.print:'p3.x =' and: p3.x;
p3.print:'p3.y =' and: p3.y;
p3.print:'p3.z =' and: p3.z;

"* Test 1: Simple assignments, variables, and operators
x = 10;
y = 20;
z = x + y;
.print: 'z = x + y =' and: z;

"* Test 2: List destructuring
a b *rest = #(100 200 300 400 500);
.print: 'a =' and: a;
.print: 'b =' and: b;
.print: 'rest =' and: rest;

"* Test 3: Lexical scopes and blocks/closures
make_counter = { |initial|
  count = initial;
  {
    count = count + 1;
    count
  }
};

counter = make_counter.value: 10;
c1 = counter.value;
c2 = counter.value;
.print: 'c1 =' and: c1;
.print: 'c2 =' and: c2;

{ |x| x.print }.value:42;

"* Test 4: Unary operators
flag = true;
inv_flag = !flag;
.print: 'flag =' and: flag and: 'inv_flag =' and: inv_flag;

num = 50;
neg_num = -num;
.print: 'num =' and: num and: 'neg_num =' and: neg_num;

"* Test 5: Dicts & Regex
my_dict = #{ 'foo': 100 'bar': 200 };
.print: 'dict =' and: my_dict;

list = #(1 2 3 4 5);
.print:'Top half =' and:list.select:{|n| n > 3};

re = #/^[a-z]+$/;
is_match = re.regex_match: 'gemini';
.print: 'regex match =' and: is_match;

"* Test 6: Non-local return (^^)
Point <-- {
  test_nlr -> {
    bar_func = { |blk|
      blk.value;
      .print: 'Inside bar: should NOT reach here!';
      111
    };

    nested_block = {
      ^^ 777
    };
    bar_func.value: nested_block;
    .print: 'Inside foo: should NOT reach here!';
    222
  }
};

.print: 'New Point=' and:Point.new;

result = p1.test_nlr;
.print: 'Result of non-local return =' and: result;

"* Test 7: Namespaces


file = [IO]File.open: '/etc/zshrc';
.print: 'file path =' and: file.path;
.print: 'file class =' and: file.class;
.print: 'file class name =' and: file.class.name;
.print: '[/]Object =' and: [/]Object;
.print: 'Object == [/]Object =' and: (Object == [/]Object);

[IO]Stdout = 'standard output';
.print: 'Stdout =' and: [IO]Stdout;

folder = [IO]Folder.open: 'bblib/tests/';
.print:'Test files =' and:folder.list;
