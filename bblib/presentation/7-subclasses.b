Person <- Employee <- { |@title|
    init: --> { |title|
        @title = title
    }

    title -> { @title }

    s --> { %'%{@name}, %{@title}, %{@age} years old' }
}

damonEmployee = Employee.new:{
    name = 'Damon'
    age = 39       "* Narrator: Still not 39
    title = 'Lead Software Engineer'
}

damonEmployee.s.puts   "* Damon, Lead Software Engineering, 39 years old
"* Damon, Lead Software Engineering, 39 years old
