// vim: foldmarker=<([{,}])> foldmethod=marker

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenTree};
use quote::*;
use syn::buffer::Cursor;
use syn::parse::discouraged::Speculative;
use syn::parse::*;
use syn::punctuated::Punctuated;
use syn::token::Colon;
use syn::*;

// bevy_exec <([{
/// Parses the following syntax:
///
///     bevy_exec! {
///         $DEST <- $VARIANT { ... }
///     }
///
/// For example:
///
///     bevy_exec! {
///         chan <- SyncEntity { entity, transform: Some(transform), appearance: None }
///         ctx.chan <- SyncEntity  { entity, transform: Some(transform), appearance: None }
///     }
struct SyntaxPair {
    sender: SenderType,
    variant: Ident,
    es: ExprStruct,
}

impl Parse for SyntaxPair {
    fn parse(input: ParseStream) -> Result<Self> {
        let fork = input.fork();
        let left = prefix(&fork);
        let id = parse::<Ident>(left.clone());
        let sender = if id.is_ok() {
            SenderType::Ident(id?)
        } else {
            let ef = parse::<ExprField>(left);
            SenderType::ExprField(ef?)
        };
        input.advance_to(&fork);
        let es: ExprStruct = input.parse()?;
        let variant = es.path.segments.first().unwrap().ident.clone();
        Ok(SyntaxPair { sender, variant, es })
    }
}

#[proc_macro]
pub fn bevy_exec(input: TokenStream) -> TokenStream {
    let SyntaxPair { sender, variant, es } = parse_macro_input!(input as SyntaxPair);

    let expanded = quote! {
        let ptr = Box::into_raw(Box::new(TaskAttachment::#es)).expose_provenance();
        #sender.send(TaskPayload::#variant{ptr}).unwrap();
    };

    TokenStream::from(expanded)
}
// }])>

#[proc_macro]
pub fn bevy_delete(input: TokenStream) -> TokenStream {
    bevy_exec(input)
}

// bevy_exec_ret <([{
/// Parses the following syntax:
///
///     bevy_exec_ret! {
///         $DEST <- $VARIANT { ... }
///     }
///
/// For example:
///
///     let handle = bevy_exec_ret! {
///         s <- RegisterMesh { mesh: mesh.into(), resp: resp_tx }
///         ctx.chan <- RegisterMesh { mesh: mesh.into(), resp: resp_tx }
///     }
#[proc_macro]
pub fn bevy_exec_ret(input: TokenStream) -> TokenStream {
    let SyntaxPair { sender, variant, mut es } = parse_macro_input!(input as SyntaxPair);

    let ident = Ident::new("resp_tx", Span::call_site());
    let ps = PathSegment { ident, arguments: PathArguments::None };
    let mut seg = Punctuated::new();
    seg.push(ps);
    let fv: FieldValue = FieldValue {
        attrs: Vec::new(),
        member: Member::Named(Ident::new("resp", Span::call_site())),
        colon_token: Some(Colon { spans: [Span::call_site(); 1] }),
        expr: Expr::Path(ExprPath {
            attrs: Vec::new(),
            qself: None,
            path: Path { leading_colon: None, segments: seg },
        }),
    };
    es.fields.push(fv);

    let expanded = quote! {
        {
            let (resp_tx, resp_rx) = oneshot::channel();
            let ptr = Box::into_raw(Box::new(TaskAttachment::#es)).expose_provenance();
            #sender.send(TaskPayload::#variant{ptr}).unwrap();
            resp_rx.await.unwrap()
        }
    };

    TokenStream::from(expanded)
}
// }])>

#[proc_macro]
pub fn bevy_new(input: TokenStream) -> TokenStream {
    bevy_exec_ret(input)
}

// utils <([{
fn create_stream(begin: Cursor, end: Cursor) -> TokenStream {
    assert!(begin <= end);

    let mut cursor = begin;
    let mut tokens = proc_macro2::TokenStream::new();
    while cursor < end {
        let (token, next) = cursor.token_tree().unwrap();
        tokens.extend(std::iter::once(token));
        cursor = next;
    }
    proc_macro::TokenStream::from(tokens)
}

fn prefix(input: ParseStream) -> TokenStream {
    input
        .step(|cursor| {
            let mut rest = *cursor;
            let head = *cursor;
            let mut first = *cursor;
            let mut next_to = false;
            while let Some((tt, next)) = rest.token_tree() {
                match &tt {
                    TokenTree::Punct(punct) if punct.as_char() == '<' => {
                        next_to = true;
                        rest = next;
                    }
                    TokenTree::Punct(punct) if punct.as_char() == '-' && next_to => {
                        let left = create_stream(head, first);
                        return Ok((left, next));
                    }
                    _ => {
                        rest = next;
                        first = next;
                        next_to = false;
                    }
                }
            }
            Err(cursor.error("no `<-` is found"))
        })
        .unwrap()
}

enum SenderType {
    Ident(Ident),
    ExprField(ExprField),
}

impl ToTokens for SenderType {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            SenderType::Ident(id) => tokens.append(id.clone()),
            SenderType::ExprField(ef) => ef.to_tokens(tokens),
        }
    }
}
// }])>

// payload_to_attachment, intern macro <([{
struct P2A {
    attachment: ExprStruct,
    ptr: Ident,
}

impl Parse for P2A {
    fn parse(input: ParseStream) -> Result<Self> {
        let fork = input.fork();
        let left = prefix(&fork);
        let attachment = parse::<ExprStruct>(left.clone())?;
        input.advance_to(&fork);
        let ptr: Ident = input.parse()?;
        Ok(P2A { attachment, ptr })
    }
}

#[proc_macro]
pub fn payload_to_attachment(input: TokenStream) -> TokenStream {
    let P2A { attachment, ptr } = parse_macro_input!(input as P2A);

    let expanded = quote! {
        // Safety: we're responsible for freeing memory.
        let ptr = unsafe { *Box::<TaskAttachment>::from_raw(std::ptr::with_exposed_provenance_mut(#ptr)) };
        let #attachment = ptr else { panic!(""); };
    };

    TokenStream::from(expanded)
}
// }])>

// User-defined event to Rhai::Map <([{
/// #derive[RhaiMap]
/// struct XEvent { .. }
///
/// // the proc macro is expanded to
/// impl FromXEvent> for rhai::Map { .. }
///
/// Only a simple struct is supported, no nested struct, no generics.
#[proc_macro_derive(RhaiMap)]
pub fn derive_rhai_map(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let struct_name = &input.ident;

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => panic!("RhaiMap supports named-field"),
        },
        _ => panic!("RhaiMap supports struct"),
    };

    let mut field_conversions_from = Vec::new();

    for field in fields.iter() {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();

        field_conversions_from.push(quote! {
            map.insert(
                #field_name_str.into(),
                rhai::Dynamic::from(value.#field_name)
            );
        });
    }

    let expanded = quote! {

        impl From<#struct_name> for rhai::Map {
            fn from(value: #struct_name) -> Self {
                let mut map: rhai::Map = rhai::Map::new();
                #(#field_conversions_from)*
                map
            }
        }

    };

    TokenStream::from(expanded)
}
// }])>

// collect trait method <([{
#[proc_macro_attribute]
pub fn scan_methods(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_trait = parse_macro_input!(item as ItemTrait);
    let trait_name = &input_trait.ident;

    // 生成 Proxy 结构体的名字
    let proxy_struct_name = format_ident!("{}ToRhai", trait_name);

    // --- 修改开始 ---
    // 我们不再存储元组，而是存储三个独立的 Vec，保持索引同步
    let mut method_names = Vec::new();
    let mut method_inputs = Vec::new();
    let mut method_outputs = Vec::new();

    for item in input_trait.items.iter() {
        if let TraitItem::Fn(method) = item {
            method_names.push(&method.sig.ident);
            method_inputs.push(&method.sig.inputs);
            method_outputs.push(&method.sig.output);
        }
    }
    // --- 修改结束 ---

    // 生成代码
    let expanded = quote! {
        // 原始 Trait
        #input_trait

        // 生成的 Proxy 结构体
        #[derive(Clone, Debug)]
        pub struct #proxy_struct_name(*mut Rhai);


        pub fn get_method_names() -> Vec<String> {
            vec![
                #( stringify!(#method_names).to_string() ),*
            ]
        }

        // 实现 Trait
        impl #trait_name for #proxy_struct_name {
            // 这里我们利用 quote 的特性：
            // 当多个变量都是集合时，#( #var1 #var2 )* 会自动按索引配对展开
            #(
                fn #method_names(#method_inputs) #method_outputs {
                    // let rhai = unsafe { &mut *self.0 };
                    // let m: Map = event.into();
                    // let ret: Dynamic = rhai.call(#method_names, (m,))?;
                    todo!()
                }
            )*
        }
    };

    TokenStream::from(expanded)
}
// }])>
