# Slop Detection Guide (Reference)

This is a reference list of common \"AI slop\" patterns to avoid. Use judgment: context matters.

## Text Slop Patterns

### High-risk phrases (nearly always slop)

- \"delve into\"
- \"dive deep into\"
- \"unpack\"
- \"navigate the complexities\"
- \"in the ever-evolving landscape\"
- \"in today's fast-paced world\"
- \"in today's digital age\"
- \"at the end of the day\"
- \"it's important to note that\"
- \"it's worth noting that\"
- \"in conclusion\" (when it's obvious)

### Medium-risk phrases (often slop)

- \"however, it is important to\"
- \"furthermore\"
- \"moreover\"
- \"in essence\"
- \"essentially\"
- \"fundamentally\"
- \"ultimately\"
- \"that being said\"

### Generic hedge language (excessive hedging)

- \"may or may not\"
- \"could potentially\"
- \"might possibly\"
- \"it appears that\"
- \"it seems that\"
- \"one could argue\"
- \"some might say\"
- \"to a certain extent\"
- \"in some cases\"
- \"generally speaking\"

### Unnecessary meta-commentary

- \"In this article, I will discuss...\"
- \"As we explore...\"
- \"Let's take a closer look...\"
- \"Now that we've covered...\"
- \"Before we proceed...\"
- \"It's crucial to understand...\"
- \"We must consider...\"

### Corporate buzzword clusters

- \"synergistic\"
- \"holistic approach\"
- \"paradigm shift\"
- \"game-changer\"
- \"revolutionary\"
- \"cutting-edge\" (when not literal)
- \"next-generation\"
- \"world-class\"
- \"best-in-class\"
- \"leverage\" (when \"use\" works)
- \"utilize\" (when \"use\" works)

### Redundant qualifiers / empty intensifiers

- \"past history\", \"future plans\", \"final outcome\"
- overuse of \"very\", \"really\", \"extremely\", \"incredibly\", \"actually\"

### Filler constructions (wordy phrases)

- \"in order to\" → \"to\"
- \"due to the fact that\" → \"because\"
- \"at this point in time\" → \"now\"
- \"for the purpose of\" → \"for\"
- \"has the ability to\" / \"is able to\" → \"can\"

## Design Slop Patterns (UI/UX)

- Decorative gradients and effects as the primary design element
- Generic landing-page template structure instead of content-first layout
- Card overuse (cards within cards, everything boxed)
- Center-alignment everywhere for long text
- Icon-only controls without labels or accessible names
- Animations everywhere (fade-in-on-scroll, parallax on everything)

## Code Slop Patterns (General)

### Naming slop

- overly generic names: `data`, `result`, `temp`, `value`, `item`, `thing`, `obj`, `info`
- placeholder names: `foo`, `bar`, `baz`, `test1`, `MyClass`
- suffix soup: `Helper`, `Manager`, `Handler` without specificity

### Comment slop

- comments that restate syntax instead of intent
- TODOs with no owner/action/why (be specific if TODO is unavoidable)
- big section divider comments instead of functions/modules

### Structure/implementation slop

- unnecessary abstraction layers (factory/singleton/etc.) without need
- generic error handling (catch-all, swallow errors)
- copy/paste variants
- magic numbers with no explanation
- premature optimization that harms clarity

