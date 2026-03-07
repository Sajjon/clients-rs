use proc_macro::{Delimiter, Spacing, TokenStream, TokenTree};

#[proc_macro]
pub fn client(input: TokenStream) -> TokenStream {
    match expand_client(input) {
        Ok(stream) => stream,
        Err(message) => compile_error(message),
    }
}

#[proc_macro_derive(Depends, attributes(dep))]
pub fn derive_depends(input: TokenStream) -> TokenStream {
    match derive_depends_impl(input) {
        Ok(stream) => stream,
        Err(message) => compile_error(message),
    }
}

fn expand_client(input: TokenStream) -> Result<TokenStream, String> {
    let tokens = input.into_iter().collect::<Vec<_>>();
    let struct_index = tokens
        .iter()
        .position(|token| is_ident(token, "struct"))
        .ok_or_else(|| "client! expects `struct`".to_string())?;

    let visibility = tokens_to_string(&tokens[..struct_index]);
    let name = ident_at(&tokens, struct_index + 1, "a client name")?;

    if !matches!(tokens.get(struct_index + 2), Some(token) if is_ident(token, "as")) {
        return Err("client! expects `as <module_name>` after the struct name".into());
    }

    let module = ident_at(&tokens, struct_index + 3, "a module name after `as`")?;
    let body = match tokens.get(struct_index + 4) {
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => group.stream(),
        _ => return Err("client! expects a braced method body".into()),
    };

    if tokens.len() != struct_index + 5 {
        return Err("unexpected tokens after the client body".into());
    }

    let methods = parse_methods(body)?;
    if methods.is_empty() {
        return Err("client! requires at least one method".into());
    }

    let visibility_prefix = with_trailing_space(&visibility);
    let field_lines = methods
        .iter()
        .map(Method::render_field)
        .collect::<Vec<_>>()
        .join("\n");
    let method_lines = methods
        .iter()
        .map(Method::render_method)
        .collect::<Vec<_>>()
        .join("\n\n");
    let live_lines = methods
        .iter()
        .map(|method| format!("{}: {}", method.name, method.render_live_initializer(&module)))
        .collect::<Vec<_>>()
        .join(",\n                    ");
    let module_lines = methods
        .iter()
        .map(|method| method.render_module(&name))
        .collect::<Vec<_>>()
        .join("\n");

    let output = format!(
        "#[derive(Clone, Copy)]
        {visibility_prefix}struct {name} {{
            {field_lines}
        }}

        impl {name} {{
            {method_lines}
        }}

        impl ::dep::Dependency for {name} {{
            fn live() -> Self {{
                Self {{
                    {live_lines}
                }}
            }}
        }}

        impl ::core::default::Default for {name} {{
            fn default() -> Self {{
                <Self as ::dep::Dependency>::live()
            }}
        }}

        {visibility_prefix}mod {module} {{
            use super::*;

            pub fn get() -> super::{name} {{
                ::dep::get::<super::{name}>()
            }}

            {module_lines}
        }}"
    );

    output
        .parse::<TokenStream>()
        .map_err(|error| error.to_string())
}

#[derive(Clone)]
struct Method {
    name: String,
    visibility: String,
    arguments: Vec<Argument>,
    return_ty: String,
    implementation: Option<String>,
    is_async: bool,
}

#[derive(Clone)]
struct Argument {
    name: String,
    ty: String,
}

impl Method {
    fn arity(&self) -> usize {
        self.arguments.len()
    }

    fn eraser_name(&self) -> String {
        if self.is_async {
            format!("::dep::erase_async_{}", self.arity())
        } else {
            format!("::dep::erase_sync_{}", self.arity())
        }
    }

    fn args_decl(&self) -> String {
        self.arguments
            .iter()
            .map(|argument| format!("{}: {}", argument.name, argument.ty))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn args_types(&self) -> String {
        self.arguments
            .iter()
            .map(|argument| argument.ty.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn args_names(&self) -> String {
        self.arguments
            .iter()
            .map(|argument| argument.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn fn_pointer_return(&self) -> String {
        if self.is_async {
            format!("::dep::BoxFuture<{}>", self.return_ty)
        } else {
            self.return_ty.clone()
        }
    }

    fn render_field(&self) -> String {
        format!(
            "{}: fn({}) -> {},",
            self.name,
            self.args_types(),
            self.fn_pointer_return()
        )
    }

    fn render_method(&self) -> String {
        let visibility = with_trailing_space(&self.visibility);
        let args_decl = self.args_decl();
        let call_args = self.args_names();

        if self.is_async {
            format!(
                "{visibility}async fn {}(&self{}{}) -> {} {{
                (self.{})({}).await
            }}",
                self.name,
                maybe_comma(&args_decl),
                args_decl,
                self.return_ty,
                self.name,
                call_args,
            )
        } else {
            format!(
                "{visibility}fn {}(&self{}{}) -> {} {{
                (self.{})({})
            }}",
                self.name,
                maybe_comma(&args_decl),
                args_decl,
                self.return_ty,
                self.name,
                call_args,
            )
        }
    }

    fn render_live_initializer(&self, module: &str) -> String {
        if let Some(implementation) = &self.implementation {
            format!("{}({implementation})", self.eraser_name())
        } else if self.is_async {
            format!(
                "{{
                        fn __dep_unimplemented({}) -> ::dep::BoxFuture<{}> {{
                            ::dep::boxed(async move {{
                                ::dep::unimplemented_dependency(\"{}.{}\")
                            }})
                        }}

                        __dep_unimplemented
                    }}",
                self.args_decl(),
                self.return_ty,
                module,
                self.name,
            )
        } else {
            format!(
                "{{
                        fn __dep_unimplemented({}) -> {} {{
                            ::dep::unimplemented_dependency(\"{}.{}\")
                        }}

                        __dep_unimplemented
                    }}",
                self.args_decl(),
                self.return_ty,
                module,
                self.name,
            )
        }
    }

    fn render_module(&self, client_name: &str) -> String {
        let args_types = self.args_types();
        let fn_pointer_return = self.fn_pointer_return();
        let eraser = self.eraser_name();

        if self.is_async {
            format!(
                "pub mod {} {{
                    use super::*;

                    pub fn get() -> fn({}) -> {} {{
                        super::get().{}
                    }}

                    pub fn override_with<F, Fut>(builder: &mut ::dep::OverrideBuilder, implementation: F)
                    where
                        F: Fn({}) -> Fut + Copy + 'static,
                        Fut: ::core::future::Future<Output = {}> + Send + 'static,
                    {{
                        builder.update::<super::super::{client_name}, _>(|mut dependency| {{
                            dependency.{} = {}(implementation);
                            dependency
                        }});
                    }}
                }}",
                self.name,
                args_types,
                fn_pointer_return,
                self.name,
                args_types,
                self.return_ty,
                self.name,
                eraser,
            )
        } else {
            format!(
                "pub mod {} {{
                    use super::*;

                    pub fn get() -> fn({}) -> {} {{
                        super::get().{}
                    }}

                    pub fn override_with<F>(builder: &mut ::dep::OverrideBuilder, implementation: F)
                    where
                        F: Fn({}) -> {} + Copy + 'static,
                    {{
                        builder.update::<super::super::{client_name}, _>(|mut dependency| {{
                            dependency.{} = {}(implementation);
                            dependency
                        }});
                    }}
                }}",
                self.name,
                args_types,
                fn_pointer_return,
                self.name,
                args_types,
                self.return_ty,
                self.name,
                eraser,
            )
        }
    }
}

fn parse_methods(stream: TokenStream) -> Result<Vec<Method>, String> {
    split_top_level(stream, ';')
        .into_iter()
        .map(|tokens| parse_method(&tokens))
        .collect()
}

fn parse_method(tokens: &[TokenTree]) -> Result<Method, String> {
    if tokens.is_empty() {
        return Err("empty method definition".into());
    }

    let fn_index = tokens
        .iter()
        .position(|token| is_ident(token, "fn"))
        .ok_or_else(|| "client methods must use `fn`".to_string())?;

    let mut leading = tokens[..fn_index].iter().cloned().collect::<Vec<_>>();
    let is_async = matches!(
        leading.last(),
        Some(TokenTree::Ident(ident)) if ident.to_string() == "async"
    );
    if is_async {
        leading.pop();
    }

    let visibility = tokens_to_string(&leading);
    let name = ident_at(tokens, fn_index + 1, "a method name")?;
    let arguments_group = match tokens.get(fn_index + 2) {
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => group.stream(),
        _ => return Err(format!("method `{name}` is missing its argument list")),
    };

    let rest = &tokens[fn_index + 3..];
    if !matches!(rest.first(), Some(TokenTree::Punct(punct)) if punct.as_char() == '-')
        || !matches!(rest.get(1), Some(TokenTree::Punct(punct)) if punct.as_char() == '>')
    {
        return Err(format!("method `{name}` is missing `->`"));
    }

    let eq_index = rest.iter().position(|token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == '='));
    let return_tokens = match eq_index {
        Some(index) => &rest[2..index],
        None => &rest[2..],
    };
    if return_tokens.is_empty() {
        return Err(format!("method `{name}` is missing a return type"));
    }

    let implementation = eq_index.map(|index| tokens_to_string(&rest[index + 1..]));
    let arguments = parse_arguments(arguments_group)?;
    if arguments.len() > 4 {
        return Err(format!(
            "method `{name}` has {} arguments, but only up to 4 are supported right now",
            arguments.len()
        ));
    }

    Ok(Method {
        name,
        visibility,
        arguments,
        return_ty: tokens_to_string(return_tokens),
        implementation,
        is_async,
    })
}

fn parse_arguments(stream: TokenStream) -> Result<Vec<Argument>, String> {
    split_top_level(stream, ',')
        .into_iter()
        .map(|tokens| {
            let colon_index = tokens
                .iter()
                .position(|token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == ':'))
                .ok_or_else(|| "expected arguments to look like `name: Type`".to_string())?;

            let name = tokens[..colon_index]
                .iter()
                .rev()
                .find_map(|token| match token {
                    TokenTree::Ident(ident) => Some(ident.to_string()),
                    _ => None,
                })
                .ok_or_else(|| "expected an argument name".to_string())?;

            let ty = tokens_to_string(&tokens[colon_index + 1..]);
            if ty.is_empty() {
                return Err("expected an argument type".into());
            }

            Ok(Argument { name, ty })
        })
        .collect()
}

fn derive_depends_impl(input: TokenStream) -> Result<TokenStream, String> {
    let mut tokens = input.into_iter().peekable();

    while let Some(token) = tokens.next() {
        if is_ident(&token, "struct") {
            return expand_struct(tokens);
        }
    }

    Err("Depends can only be derived for structs".into())
}

fn expand_struct<I>(mut tokens: I) -> Result<TokenStream, String>
where
    I: Iterator<Item = TokenTree>,
{
    let name = match tokens.next() {
        Some(TokenTree::Ident(ident)) => ident,
        _ => return Err("expected a struct name".into()),
    };

    let fields_group = loop {
        match tokens.next() {
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => break group,
            Some(_) => {
                return Err("Depends does not support generics or where clauses yet".into());
            }
            None => return Err("expected a braced struct body".into()),
        }
    };

    let fields = parse_fields(fields_group.stream())?;
    let initializers = fields
        .into_iter()
        .map(|field| {
            if field.injected {
                format!("{}: ::dep::get::<{}>()", field.name, field.ty)
            } else {
                format!("{}: ::core::default::Default::default()", field.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let output = format!(
        "impl ::core::default::Default for {name} {{
            fn default() -> Self {{
                Self {{ {initializers} }}
            }}
        }}

        impl {name} {{
            pub fn from_deps() -> Self {{
                ::core::default::Default::default()
            }}
        }}",
    );

    output
        .parse::<TokenStream>()
        .map_err(|error| error.to_string())
}

struct Field {
    name: String,
    ty: String,
    injected: bool,
}

fn parse_fields(stream: TokenStream) -> Result<Vec<Field>, String> {
    split_top_level(stream, ',')
        .into_iter()
        .map(|tokens| parse_field(&tokens))
        .collect()
}

fn parse_field(tokens: &[TokenTree]) -> Result<Field, String> {
    let mut injected = false;
    let mut colon_index = None;

    for (index, token) in tokens.iter().enumerate() {
        if matches_dep_attribute(tokens, index) {
            injected = true;
        }

        if let TokenTree::Punct(punct) = token {
            if punct.as_char() == ':' {
                colon_index = Some(index);
                break;
            }
        }
    }

    let colon_index = colon_index.ok_or_else(|| "expected a named struct field".to_string())?;

    let name = tokens[..colon_index]
        .iter()
        .rev()
        .find_map(|token| match token {
            TokenTree::Ident(ident) => Some(ident.to_string()),
            _ => None,
        })
        .ok_or_else(|| "expected a field name".to_string())?;

    let ty_tokens = tokens[colon_index + 1..].iter().cloned().collect::<TokenStream>();
    if ty_tokens.is_empty() {
        return Err("expected a field type".into());
    }

    Ok(Field {
        name,
        ty: ty_tokens.to_string(),
        injected,
    })
}

fn split_top_level(stream: TokenStream, separator: char) -> Vec<Vec<TokenTree>> {
    let mut items = Vec::new();
    let mut current = Vec::new();

    for token in stream {
        match &token {
            TokenTree::Punct(punct)
                if punct.as_char() == separator && punct.spacing() == Spacing::Alone =>
            {
                if !current.is_empty() {
                    items.push(current);
                    current = Vec::new();
                }
            }
            _ => current.push(token),
        }
    }

    if !current.is_empty() {
        items.push(current);
    }

    items
}

fn ident_at(tokens: &[TokenTree], index: usize, expected: &str) -> Result<String, String> {
    match tokens.get(index) {
        Some(TokenTree::Ident(ident)) => Ok(ident.to_string()),
        _ => Err(format!("expected {expected}")),
    }
}

fn matches_dep_attribute(tokens: &[TokenTree], index: usize) -> bool {
    let Some(TokenTree::Punct(pound)) = tokens.get(index) else {
        return false;
    };
    if pound.as_char() != '#' {
        return false;
    }

    let Some(TokenTree::Group(group)) = tokens.get(index + 1) else {
        return false;
    };

    if group.delimiter() != Delimiter::Bracket {
        return false;
    }

    let mut attribute_tokens = group.stream().into_iter();
    matches!(attribute_tokens.next(), Some(TokenTree::Ident(ident)) if ident.to_string() == "dep")
}

fn is_ident(token: &TokenTree, expected: &str) -> bool {
    matches!(token, TokenTree::Ident(ident) if ident.to_string() == expected)
}

fn tokens_to_string(tokens: &[TokenTree]) -> String {
    tokens.iter().map(TokenTree::to_string).collect::<Vec<_>>().join(" ")
}

fn with_trailing_space(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        format!("{value} ")
    }
}

fn maybe_comma(value: &str) -> &'static str {
    if value.is_empty() {
        ""
    } else {
        ", "
    }
}

fn compile_error(message: String) -> TokenStream {
    format!("compile_error!({message:?});")
        .parse()
        .expect("compile_error! should parse")
}
