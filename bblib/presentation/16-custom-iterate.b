"* This exists in the standard library as NumberRange.
MyRange <- { |@start @end @current|
    .mix:Iterate;

    init: -> { |start end|
        @current = start-1; @start = start; @end = end
    }

    "* Only one method required to implement Iterate:
    next -> {
        (@current >= @end-1).if:{nil} else:{@current = @current + 1}
    }
}

r = MyRange.new:{
    start = 0
    end = 9
}

r.collect:{ |n| n+1 }
"* #( 1 2 3 4 5 6 7 8 9 )
