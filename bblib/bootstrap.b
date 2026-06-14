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

    jsonRep -> { self }
};

false <-- {
    s --> { 'false' };

    if: -> { |ifblock| nil };
    else: -> { |elseblock| elseblock.value };
    if:else: -> { |_ elseblock| elseblock.value };
    not -> { true }
    #'!' -> { true }

    jsonRep -> { self }
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
    double -> { self }

    next -> { self + 1.0 }

    #'-' -> { 0.0 - self }
    #'+' -> { self }

    abs -> { (self < 0).if:{ -self } else:{ self } }

    jsonRep -> { self };

    .meta <-- {
        default -> { 0.0 }
    }
};

Integer <-- {
    integer -> { self }

    next -> { self + 1 }

    #'-' -> { 0 - self }
    #'+' -> { self }

    abs -> { (self < 0).if:{ -self } else:{ self } }

    jsonRep -> { self };

    .meta <-- {
        default -> { 0 }
    }
};

Nil <-- {
    s --> { '' }
    defined? --> { false }

    #'+:' -> { |_| self }
    #'-:' -> { |_| self }
    #'*:' -> { |_| self }
    #'/:' -> { |_| self }
    #'%:' -> { |_| self }

    jsonRep -> { self };

    .meta <-- {
        default -> { nil }
    }
};

ANSI <- { |@string|
    .can:ActAsUserString;

    init: -> { |string|
        @string = string
    }

    string -> { @string }

    "* XXX: Need a way to modify scope for this to work.
    "* #'%' -> { .class.newUserString:(%@string) }

    #'%:' -> { |arg| .class.newUserString:(@string % arg) }

    "* XXX: Rendered length?
    length -> { @string.length }

    s --> { '#ANSI\'' + @string + '\'' };

    .meta <-- {
        default -> { #ANSI'' }

        newUserString: -> { |s:String| ^.new:{ string = s } }
    }
};
