(function() {var implementors = {};
implementors["num_rational"] = [{"text":"impl&lt;T:&nbsp;Clone + Integer&gt; Zero for Ratio&lt;T&gt;","synthetic":false,"types":[]}];
implementors["num_traits"] = [];
implementors["uom"] = [{"text":"impl&lt;D:&nbsp;?Sized, U:&nbsp;?Sized, V&gt; Zero for Quantity&lt;D, U, V&gt; <span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;D: Dimension,<br>&nbsp;&nbsp;&nbsp;&nbsp;D::Kind: Add,<br>&nbsp;&nbsp;&nbsp;&nbsp;U: Units&lt;V&gt;,<br>&nbsp;&nbsp;&nbsp;&nbsp;V: Num + Conversion&lt;V&gt;,&nbsp;</span>","synthetic":false,"types":[]}];
if (window.register_implementors) {window.register_implementors(implementors);} else {window.pending_implementors = implementors;}})()