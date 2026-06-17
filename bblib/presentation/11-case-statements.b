Case <- {
    .meta <-- {
        for:with: -> { |obj block:Block|
            {
                block.value:obj withSelf:.new:{ subject = obj }
            }.catch:{|r| ^^r }
        };
    };

    init: -> {|subject| @subject = subject };

    when:do: -> { |cond block:Block|
        (cond ~ @subject).if:{
            (block.value:@subject withSelf:@subject).throw
        }
    };

    when:do: --> { |cond value:Object|
        (cond ~ @subject).if:{
            value.throw
        }
    };

    default: -> { |block|
        block.value.throw
    };
};

Object <-- {
    case: -> { |block| Case.for:self with:block };
};
