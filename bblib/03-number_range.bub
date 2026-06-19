NumberRange <- { | @start @end @n |
    .sealed!;

    .can:Iterate;

    init: -> {|start end|
        @start = start
        @end = end

        @n = (@start > @end).if: { -1 }
                           else: { 1 }
    }

    s --> { 'NumberRange(' + @start + '..' + @end + ')' }

    #'~:' --> { |n|
        (@start > @end).if:{ (n <= @start) && (n > @end) }
                      else:{ (n >= @start) && (n < @end) }
    }

    each: -> { |b|
        i = @start;
        { (@n > 0).if:{ i < @end } else:{ i > @end } }.whileDo:{
            b.valueWithSelfOrArg:i;
            i = i + @n;
        }
    }
};

Integer <-- {
    #'..:' -> { |e| NumberRange.new:{ start=self; end=e } }
};

Double <-- {
    #'..:' -> { |e| NumberRange.new:{ start=self; end=e } }
};
