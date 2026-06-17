Person <- { |@name @age|
    init: -> { |name age|
        @name = name
        @age = age
    }

    name -> { @name }
    age -> { @age }

    s --> { %'%{@name}, %{@age} years old' }
}

damonPerson = Person.new:{
    name = 'Damon'
    age = 39       "* Narrator: He's not 39
}

damonEmployee.s.puts
"* Damon, 39 years old
