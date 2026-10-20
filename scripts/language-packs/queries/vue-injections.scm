; Unqualified script/style blocks.
((script_element (start_tag) @_start (raw_text) @injection.content)
 (#not-match? @_start "(?i)\\blang\\s*=")
 (#set! injection.language "javascript"))
((style_element (start_tag) @_start (raw_text) @injection.content)
 (#not-match? @_start "(?i)\\blang\\s*=")
 (#set! injection.language "css"))

; Explicit language attributes (js/ts/tsx/css/scss use the installed registry).
((script_element
   (start_tag (attribute (attribute_name) @_name
     (quoted_attribute_value (attribute_value) @injection.language)))
   (raw_text) @injection.content)
 (#eq? @_name "lang"))
((style_element
   (start_tag (attribute (attribute_name) @_name
     (quoted_attribute_value (attribute_value) @injection.language)))
   (raw_text) @injection.content)
 (#eq? @_name "lang"))
((interpolation (raw_text) @injection.content)
 (#set! injection.language "javascript"))
((directive_attribute (quoted_attribute_value (attribute_value) @injection.content))
 (#set! injection.language "javascript"))
