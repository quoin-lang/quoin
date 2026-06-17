"
Basic stuff that needs to be loaded first. Everything else should try to be self-contained within its .b file.
"

true <-- {
    s --> { 'true' };

    if: -> { |ifblock| ifblock.value };
    else: -> { |_| nil };
    if:else: -> { |ifblock _| ifblock.value };
    not -> { false }
    #'!' -> { false }
};

false <-- {
    s --> { 'false' };

    if: -> { |ifblock| nil };
    else: -> { |elseblock| elseblock.value };
    if:else: -> { |_ elseblock| elseblock.value };
    not -> { true }
    #'!' -> { true }
};

Object <-- {
    defined? -> { true }
};

Mixin <- {
    .meta <-- {
        assertMeetsRequirements: -> {}
    }
}

Mixin <- ActAsUserList <- {
    .sealed!;
};

Mixin <- ActAsUserString <- {
    .sealed!;
};

Error <- {
    s --> { .class.name }

    payload -> { @errorData.payload }
    stackframes -> { @errorData.stackframes }
    format -> { @errorData.format }
};

Double <-- {
    .meta <-- {
        default -> { 0.0 }
    }

    double -> { self }

    next -> { self + 1.0 }

    abs -> { (self < 0).if:{ -self } else:{ self } }
};

Integer <-- {
    .meta <-- {
        default -> { 0 }
    }

    integer -> { self }

    next -> { self + 1 }

    abs -> { (self < 0).if:{ -self } else:{ self } }
};

Nil <-- {
    .meta <-- {
        default -> { nil }
    }

    s --> { '' }
    defined? --> { false }

    #'+:' -> { |_| self }
    #'-:' -> { |_| self }
    #'*:' -> { |_| self }
    #'/:' -> { |_| self }
    #'%:' -> { |_| self }
};

Block <-- {
    whileDo: -> { |block|
        s = self
        s.value.if:{
            block.value;
            ^^s.whileDo:block;
        };
    };

    whileDefinedDo: -> { |block|
        s = self
        v = s.value
        v.defined?.if:{
            block.value:v;
            ^^s.whileDefinedDo:block;
        };
    };
};

ANSI <- { |@string|
    .can:ActAsUserString;

    .meta <-- {
        default -> { #ANSI'' }

        newUserString: -> { |s:String| ^.new:{ string = s } }
    }

    init: -> { |string|
        @string = string
    }

    string -> { @string }

    "* XXX: Need a way to modify scope for this to work.
    "* #'%' -> { .class.newUserString:(%@string) }

    #'%:' -> { |arg| .class.newUserString:(@string % arg) }

    "* XXX: Rendered length?
    length -> { @string.length }

    s --> { '#ANSI\'' + @string + '\'' }
};
