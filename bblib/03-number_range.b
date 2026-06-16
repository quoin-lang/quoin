NumberRange <- { | @start @end @cur @n |
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

    reset -> { @cur = @start - @n }

    next -> {
        @cur.defined?.else:{ @cur = @start - @n }

        @cur = @cur + @n

        ^(((@cur-@end) < 0) == (@n < 0)).if:{ nil } else:{ @cur }
    }
};

Integer <-- {
    #'..:' -> { |e| NumberRange.new:{ start=self; end=e } }
};

Double <-- {
    #'..:' -> { |e| NumberRange.new:{ start=self; end=e } }
};
