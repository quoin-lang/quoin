String <-- {
    "
    .meta <-- {
        default -> { '' }
    }
    "

    split: -> { |pat:String| .splitString:pat }
    split: --> { |p:Regex| p.split:self }

    split -> { #/\s+/.split:self }

    jsonRep -> { self }
}
