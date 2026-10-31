# Expansão Music — 31/10/2026

## Âmbito autorizado

O proprietário substituiu o bot de testes pelo Music real. Não iniciar outra ligação gateway com o mesmo token. TTS e Helper ficam fora da alteração. Tudo gratuito, uma call por servidor, inglês, estatísticas pessoais por adesão e sem ranking global.

## Implementado

- SQLite, importação única das regras antigas, revisões de settings e exportação compatível com o runtime anterior.
- Volume/predefinições, temporizador, Session/24-7/Auto-join, recuperação condicionada e limitada.
- Gostos por pessoa/servidor, biblioteca e seleção adaptativa; My Station virtual com favoritos/descoberta e exclusão por dislikes.
- DJs, bloqueio e regras de canais, mantendo as maiorias de skip.
- Sete secções no dashboard, Quick Setup, perfis de servidor e controlo de privacidade.
- Player, painel reapresentável e ajuda/convite/suporte. Sem envio automático de feedback.

## Verificação

- Testes Rust de migração, gravação recusada, revisões concorrentes, IDs, seleção, privacidade, uploads, autenticação/CSRF, timers, callbacks e comandos.
- Teste otimizado de descodificação integral das 250 gravações do catálogo, incluindo75 novas.
- UI em 320/360/390/430/768/900/1440 px; navegação, Quick Setup, guardar/recarregar, descartar, menu/Escape e ausência de overflow.
- `cargo fmt`, Clippy com `-D warnings`, sintaxe JavaScript e revisão do diff.
- Os resultados efetivos do corte e hashes ficam no recibo de release. Resultados offline não são prova de audição real nem de uma chamada prolongada.

## Catálogo pendente — não anunciar como concluído

O utilizador autorizou agora testes no bot real com Night, Rain e Sleep:25 faixas por estação. Esta revisão acrescenta75 gravações de fontes CC0 OpenGameArt, incluindo25 misturas Rain fixas. Isso substitui a restrição anterior de publicação apenas após audição; não constitui confirmação de ausência de voz. As95 candidatas anteriores Open Lo-Fi/Suno não foram usadas. A aprovação no catálogo refere-se a origem/licença e autorização de teste, não a uma audição inexistente.

[Unverified] O manifesto Open Lo-Fi declara CC0 e origem Suno. Isso não é uma verificação independente de direitos, ausência de voz, qualidade ou chuva incorporada. As candidatas não foram descarregadas nem aprovadas para o catálogo. As versões alternativas nas páginas OpenGameArt não podem preencher quotas de gravações distintas.

Para cada nova estação: aprovar 25 gravações distintas, guardar origem/licença/hash/evidência, descodificar integralmente, medir áudio, ouvir integralmente e documentar o resultado. Só depois acrescentar ao catálogo e retirar o respetivo estado pendente. Não substituir audição por tags ou por deteção automática de voz.

## Publicação e rollback

1. Validar o artefacto Linux e os hashes do binário e catálogo. Confirmar o serviço/imagem anteriores e o mount persistente.
2. Construir a imagem candidata a partir da imagem Music anterior, substituindo apenas binário e catálogo. Fazer `--check` sem rede.
3. Copiar o JSON de produção para staging sem credenciais; executar `--check-data`, integridade SQLite e comparar as regras importadas.
4. Parar somente Music, copiar o estado quiescente e preservar `.env` em backup privado. Iniciar a imagem candidata com o mesmo volume; conferir health, gateway, comandos e assets.
5. Se falhar: parar Music, exportar as regras atuais com `--export-skip` para o JSON antigo através de substituição atómica; conservar SQLite com favoritos/estatísticas, suspender intenções antigas e restaurar apenas a imagem anterior. Não sobrepor uma base de dados antiga a novas escritas.
6. Antes de publicação futura, integrar SQLite no backup consistente existente; copiar só o ficheiro principal de uma base WAL ativa não chega.

A primeira migração não pode restaurar a call da versão antiga, porque essa versão não guardava intenção de reprodução. Usar `/play` após o primeiro corte. Reinícios posteriores usam a intenção nova, respeitando modo, permissões, presença e timers.

## Aceitação real ainda necessária

- Ouvir reprodução, mudar volume, pause/resume, transição natural e trocar de estação/My Station.
- Testar a saída ao sair a última pessoa, Auto-join, timer e recuperação após um reinício Music controlado.
- Testar duas pessoas com votos/roles diferentes, dislikes e alterações na audiência.
- Validar login partilhado e gravação real no dashboard; testar perfil de servidor sem alterar o perfil global.
- Telemóvel físico e sessão prolongada não ficam aprovados apenas por capturas ou HTTP 200.

## Localhost

`node scripts/preview.mjs` serve apenas uma fixture visual em `http://localhost:4783/music/dashboard`, sem token, gateway ou dados de produção. A página indica o ambiente local. Os testes UI usam essa fixture; não equivalem a uma gravação no serviço real.
