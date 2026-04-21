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

// collect trait method <([{
/// 属性宏：分析 Trait 方法的参数
#[proc_macro_attribute]
pub fn analyze_trait_methods(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // 1. 将输入的 TokenStream 解析为 Trait 的语法树
    let input_trait = parse_macro_input!(item as ItemTrait);

    // 获取 Trait 的名称
    let trait_name = &input_trait.ident;

    let struct_name = quote::format_ident!("{}ToRhai", trait_name);
    let mut method_impls = Vec::new();

    // 2. 遍历 Trait 中的所有项（我们只关心方法）
    for item in &input_trait.items {
        if let TraitItem::Fn(method) = item {
            let method_name = &method.sig.ident;
            println!("  ├─ 📝 方法: {}", method_name);
            let method_inputs = &method.sig.inputs; // 参数列表
            let method_output = &method.sig.output; // 返回值类型

            let mut params = Vec::new();
            let mut vcp = Vec::new();

            // 3. 遍历方法的参数
            for input in &method.sig.inputs {
                match input {
                    // 忽略 &self, self 等接收者
                    FnArg::Receiver(_) => continue,

                    FnArg::Typed(PatType { pat, ty, .. }) => {
                        // 获取参数名 (将 pat: i32 中的 pat 转为字符串)
                        let arg_name = quote!(#pat).to_string();
                        let param_name = match &**pat {
                            Pat::Ident(pat_ident) => &pat_ident.ident,
                            _ => continue, // 忽略复杂模式
                        };

                        // 获取类型
                        let type_str = quote!(#ty).to_string();

                        // 判断是否为 Struct
                        let is_struct = is_likely_struct(ty.as_ref());
                        let struct_flag = if is_struct { "✅ 是" } else { "❌ 否" };

                        println!("  │   ├─ 参数: {:<15} 类型: {:<20} 是否Struct: {}", arg_name, type_str, struct_flag);
                        if is_struct {
                            let line = quote! {
                                let #param_name: Dynamic = rhai::serde::to_dynamic(#param_name).unwrap();
                            };
                            params.push(line);
                        }
                        let cp = quote! { #param_name, };
                        vcp.push(cp);
                    }
                }
            }

            // rhai.call("on_player_die", (m, cnt)).unwrap()
            let impl_code = quote! {
                fn #method_name(#method_inputs) #method_output {
                    let rhai = unsafe { &mut *self.0 };
                    #(#params)*
                    rhai.call(stringify!(#method_name), (#(#vcp)*)).unwrap()
                }
            };
            method_impls.push(impl_code);
        }
    }

    // 4. 返回原始代码，确保代码能正常编译
    // 如果这里不返回原始代码，Trait 定义就会丢失
    quote! {
        #input_trait

        // 生成新的结构体
        #[derive(Debug, Clone)]
        pub struct #struct_name(*mut Rhai);

        // 为该结构体实现 trait
        impl #trait_name for #struct_name {
            #(#method_impls)*
        }
    }
    .into()
}

/// 辅助函数：判断一个类型是否“看起来像”一个 Struct
fn is_likely_struct(ty: &Type) -> bool {
    match ty {
        // 情况 A: 简单路径类型，如 MyStruct
        Type::Path(type_path) => {
            let path = &type_path.path;

            // 如果是单段路径（没有 ::）
            if path.segments.len() == 1 {
                let ident = &path.segments.first().unwrap().ident;
                let name = ident.to_string();

                // 排除 Rust 基本类型
                if is_primitive_type(&name) {
                    return false;
                }

                // 启发式规则：Rust 中 Struct/Enum 通常首字母大写
                // 这是一个常见的约定，虽然不是 100% 准确
                if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    return true;
                }
            }
            false
        }

        // 情况 B: 引用类型，如 &MyStruct
        Type::Reference(type_ref) => {
            // 递归检查引用的内部类型
            is_likely_struct(&type_ref.elem)
        }

        // 情况 C: 智能指针，如 Box<MyStruct> 或 Arc<MyStruct>
        Type::Path(_) => {
            // 这里可以扩展逻辑去解析泛型参数，例如提取 Box<T> 中的 T
            // 为了简化，这里暂时不处理复杂的泛型嵌套
            false
        }

        _ => false,
    }
}

/// 排除基本类型
fn is_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
            | "String"
            | "Vec"
            | "Option"
            | "Result"
            | "Box"
            | "Rc"
            | "Arc"
            | "()"
            | "None"
            | "Some"
    )
}
// }])>
