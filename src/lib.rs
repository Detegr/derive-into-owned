extern crate proc_macro;
extern crate syn;
#[macro_use]
extern crate quote;

use proc_macro::TokenStream;

#[proc_macro_derive(IntoOwned)]
pub fn into_owned(input: TokenStream) -> TokenStream {
    let source = input.to_string();

    let ast = syn::parse_derive_input(&source).unwrap();

    let expanded = impl_into_owned(&ast);

    expanded.parse().unwrap()
}

fn impl_into_owned(ast: &syn::DeriveInput) -> quote::Tokens {
    // this is based heavily on https://github.com/asajeffrey/deep-clone/blob/master/deep-clone-derive/lib.rs
    let name = &ast.ident;

    let borrowed_lifetime_params = ast.generics.lifetimes.iter().map(|alpha| quote! { #alpha });
    let borrowed_type_params = ast.generics.ty_params.iter().map(|ty| quote! { #ty });
    let borrowed_params = borrowed_lifetime_params.chain(borrowed_type_params).collect::<Vec<_>>();
    let borrowed = if borrowed_params.is_empty() {
        quote! { }
    } else {
        quote! { < #(#borrowed_params),* > }
    };


    let params = ast.generics.lifetimes.iter().map(|alpha| quote! { #alpha }).chain(ast.generics.ty_params.iter().map(|ty| { let ref ident = &ty.ident; quote! { #ident } })).collect::<Vec<_>>();
    let params = if params.is_empty() {
        quote! {}
    } else {
        quote! { < #(#params),* > }
    };

    let owned_lifetime_params = ast.generics.lifetimes.iter().map(|_| quote! { 'static });
    let owned_type_params = ast.generics.ty_params.iter().map(|ty| { let ref ident = &ty.ident; quote! { #ident } });
    let owned_params = owned_lifetime_params.chain(owned_type_params).collect::<Vec<_>>();
    let owned = if owned_params.is_empty() {
        quote! { }
    } else {
        quote! { < #(#owned_params),* > }
    };

    let into_owned = match ast.body {
        syn::Body::Struct(ref variant) => {
            let inner = ctor_fields(variant);
            quote! { #name #inner }
        },
        syn::Body::Enum(ref body) => {
            let cases = body.iter()
                .map(|case| {
                    let unqualified_ident = &case.ident;
                    let ident = quote! { #name::#unqualified_ident };
                    match case.data {
                        syn::VariantData::Struct(ref body) => {
                            let idents = body.iter()
                                .map(|field| field.ident.as_ref().unwrap())
                                .collect::<Vec<_>>();
                            let cloned = body.iter()
                                .map(|field| {
                                    let ref ident = field.ident.as_ref().unwrap();
                                    let ident = quote! { #ident };
                                    let code = FieldKind::resolve(field).move_or_clone_field(&ident);
                                    quote! { #ident: #code }
                                })
                                .collect::<Vec<_>>();
                            quote! { #ident { #(#idents),* } => #ident { #(#cloned),* } }
                        },
                        syn::VariantData::Tuple(ref body) => {
                            let idents = (0..body.len())
                                .map(|index| syn::Ident::from(format!("x{}", index)))
                                .collect::<Vec<_>>();
                            let cloned = idents.iter().zip(body.iter())
                                .map(|(ident, field)| {
                                    let ident = quote! { #ident };
                                    FieldKind::resolve(field).move_or_clone_field(&ident)
                                })
                                .collect::<Vec<_>>();
                            quote! { #ident ( #(#idents),* ) => #ident ( #(#cloned),* ) }
                        },
                        syn::VariantData::Unit => {
                            quote! { #ident => #ident }
                        },
                    }
                })
                .collect::<Vec<_>>();
            quote! { match self { #(#cases),* } }
        },
    };

    quote! {
        impl #borrowed #name #params {
            pub fn into_owned(self) -> #name #owned { #into_owned }
        }
    }
}

fn ctor_fields(data: &syn::VariantData) -> quote::Tokens {
    match *data {
        syn::VariantData::Struct(ref body) => {
            let fields = body.iter()
                .map(|field| {
                    let ident = field.ident.as_ref().unwrap();
                    let field_ref = quote! { self.#ident };
                    let code = FieldKind::resolve(field).move_or_clone_field(&field_ref);
                    quote! { #ident: #code }
                })
                .collect::<Vec<_>>();
            quote! { { #(#fields),* } }
        },
        syn::VariantData::Tuple(ref body) => {
            let fields = body.iter()
                .enumerate()
                .map(|(index, field)| {
                    let index = syn::Ident::from(index);
                    let index = quote! { self.#index };
                    FieldKind::resolve(field).move_or_clone_field(&index)
                })
                .collect::<Vec<_>>();
            quote! { ( #(#fields),* ) }
        },
        syn::VariantData::Unit => {
            quote! {}

        }
    }
}

enum FieldKind {
    PlainCow,
    AssumedCow,
    RecursiveOption(Box<FieldKind>),
    JustMoved
}

impl FieldKind {

    fn resolve(field: &syn::Field) -> Self {
        match &field.ty {
            &syn::Ty::Path(None, syn::Path { ref segments, .. }) => {
                if is_cow(segments) {
                    FieldKind::PlainCow
                } else if is_cow_alike(segments) {
                    FieldKind::AssumedCow
                } else {
                    FieldKind::JustMoved
                }
            },
            _ => FieldKind::JustMoved,
        }
    }

    fn move_or_clone_field(&self, ident: &quote::Tokens) -> quote::Tokens {
        use self::FieldKind::*;

        match self {
            &PlainCow => quote! { ::std::borrow::Cow::Owned(#ident.into_owned()) },
            &AssumedCow => quote! { #ident.into_owned() },
            &RecursiveOption(_) => unimplemented!(),
            &JustMoved => quote! { #ident },
        }
    }
}

fn is_cow(segments: &Vec<syn::PathSegment>) -> bool {
    let idents = segments.iter().map(|x| &x.ident).collect::<Vec<_>>();

    (idents.len() == 3 && idents[0] == "std" && idents[1] == "borrow" && idents[2] == "Cow")
        || (idents.len() == 2 && idents[0] == "borrow" && idents[1] == "Cow")
        || (idents.len() == 1 && idents[0] == "Cow")
}

fn is_cow_alike(segments: &Vec<syn::PathSegment>) -> bool {
    if let Some(&syn::PathParameters::AngleBracketed(ref data)) = segments.last().map(|x| &x.parameters) {
        !data.lifetimes.is_empty()
    } else {
        false
    }
}

// fn is_opt_cow(segments: &Vec<syn::PathSegment>) -> bool {
//     unimplemented!()
// }
