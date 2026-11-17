//! The `_sqlx_migrations` table written before the sqlx 0.9 upgrade (#435) must still validate:
//! sqlx refuses to start when a recorded checksum differs from the migration it embeds, so a
//! changed checksum would stop the server from booting on the production database.

use sha2::{Digest, Sha384};

/// `(version, checksum)` exactly as sqlx 0.8.6 recorded them in `_sqlx_migrations` (captured from
/// a database migrated by the server built on 0.8.6). Never edit: migrations are immutable.
const RECORDED_BY_SQLX_0_8: &[(i64, &str)] = &[
    (
        1,
        "6c37e34ee0e2f077f933853c8f7f63ddf9787d3bef14a51fae9190d7d6ef4d4a96dda41f82e6cc93736a81b08c1432a7",
    ),
    (
        2,
        "a1aa71c080ac15b129b8373938b37b888f282cedfcdf52eeb7c784ca4432856c429d33450d074c4913b38f6e82324c2a",
    ),
    (
        3,
        "50c2595721f4a1c20e6f3228f318fa0af4f5d69f9fbae76c263e9ab82bb5ff49fb82dee62f1cd803143c5f225648dce2",
    ),
    (
        4,
        "b5e6552905d7dbe59052c691d3c7244eff4f6994af9c25c53ca8979b0b5d375b7955dc48bf9103dd036108cd58c22abc",
    ),
    (
        5,
        "ad9b13b734e5d71e63cb5a2954592384066824c5881ac68df7c55e1808e46186250f3f1fdb518f318a4d8d6be780ce53",
    ),
    (
        6,
        "7bdd0728ba31dc78f2fa7966c193769014aab4846061cb34e3a940574c53a59266cd3a39dd411e2c7494652f92d48819",
    ),
    (
        7,
        "6a3a511c53dcb286d7bf7dd88b1ae2a36093bd565942777beafe9e817654b0e123416702185d3c993aebafc94ced0590",
    ),
    (
        8,
        "f60e7c6291c573082af6ad76110d7b768bc32eda332578d91667a0a46ebb7951d10feeb6c4bc52e8e09ff21570754ff8",
    ),
    (
        9,
        "e0deb36114f44f276beedc49ecc823db86b94633b70191bc3555ca5c8c3982f5397e2b0805c357590e8cb86e3890ba90",
    ),
    (
        10,
        "ad306e1158af37ed6c2277bc77db74e5c62ccac871af1c6bbf9523b457ec84a571566258b6ab15819d4a7f8eabc23096",
    ),
    (
        11,
        "3e598dceac3f9d36c11411d6f97b8f8d2d5f805810650c954136afda5de77561d737f10076d4ce884b5f64bfa14f9da4",
    ),
    (
        12,
        "a5b1da5e6d231e2d07f22851cfadda6eb7ef6f6a7293d5141f8afea5dd6ab0ddda2c8b4a133743cb6769ded54db360de",
    ),
    (
        13,
        "a9539aefacc26342c588a20ef4a077725d60c9fdd6c84599d0804fa596750ca53b5a0fdd104ab739a1fc65d9fee71b11",
    ),
    (
        14,
        "7006b954718362ab48daa702f8116b87798bb6623f020a0bb790f50704d9571135df0e75c323a262fd06f9e85d8e0e4f",
    ),
    (
        15,
        "37e27b65db9ce8b55948e7bf6acb5e9943384a4e1d69d5d2593e4afc95a0f6c4df5620b6babdb46eb050254a32a96fe5",
    ),
    (
        16,
        "6ae192a587440dfa6b891bc773edf555982e89d5147933e1d3ed8cf6cbb80b146bba8c88e3d2709fa744504a9d398412",
    ),
    (
        17,
        "2ffee1024f5c744ab0962ce164c4fdb6d511b13d64ac1014ecdf21cc9890d61635114bcc4aaf06108f5456d14084c786",
    ),
    (
        18,
        "66e4ac7ab369d111be7fbc7209579ff3230370438760d7c52372b8933786ecd3d31e88e39b75c7d4b8cd6ee25d045530",
    ),
    (
        19,
        "b8201ca2edc03e88c3b63b94470e63c9d605e05598c5fd7162379dc266ed6f96395587b1efecf2dad603f12d854eed36",
    ),
    (
        20,
        "b691d4d2962af28ac3933c85676b2432e9f71879d3da852948aa9b35c02a6998180565060fd3fef727f09b4643fcdb6c",
    ),
    (
        21,
        "2c9fe27e6fa89a5f11788bc2ef5210b5cd524d267cfe906d3b9ffabeb42964c7a63288daea09cb88ca3812899aa1235b",
    ),
    (
        22,
        "e019d2b2dd59f016ca87d71789b8b6e12aaef588ea95bcdb5f650be8f46831082aee7dd3b8afa8e5c388b11b0763437b",
    ),
    (
        23,
        "839838bb3d116980c532cb731ee413bea22d2ee112f2de620d1e4645a7ff7a3f2fba9b9501c319c4844ab1f840bc6bf4",
    ),
    (
        24,
        "a48b9c3107eeabc53908c3e595175c77971755c7195f9c511965d57ba3da4500bb69d7543bc7e55a5fbf0f5bb6fde81f",
    ),
    (
        25,
        "e58df390f84a37ce3f86b3ab89eec5e70f8992a61688dc3b4e0f4d82410a728c8221e444d5f988d6c29c7b7e81fb91af",
    ),
    (
        26,
        "738a33beef30b1c0dee4adb8953f622edd115f1486eb8fbab9fcfa2689011f46a840dff5825edc7bdba9f4b3817c3159",
    ),
    (
        27,
        "50292c31e6746a1373116106629cc8fd6da90bdf3ae5f5a5b10b6883045565a7cc976f3846ea10fd716c9d979d5529c0",
    ),
    (
        28,
        "efce16f495c844b8f66c155b5328009dd5e92a02eeb7715f45dfadbc18ecda954f7914fae1ca50cfb35bbae494150cb8",
    ),
    (
        29,
        "8955a614abd49ff777f222a14618f98a31d27e4604c64ac7251a24635fe420c6a14afff31d47468958386732a5de007b",
    ),
    (
        30,
        "c27f23d99fd74fdc9394515d18a261eecc240d82c6c6a3c5ad5be721457e6626da528ec39e5899f5d04203c95fd69629",
    ),
    (
        31,
        "b0a481aa55940244d875bc716be4ae6a042d5361fec7f91e72fdd4c187643d8bc71e6371514afba46c4ff73181a6698e",
    ),
    (
        32,
        "909181a50a8b12d97bda1e61b407c4c9b93f692d7138eccd16ba76c550603afaaa33edd91bf54b55d6e28353b26dee87",
    ),
    (
        33,
        "e7be88c59e6e84183a50867d5b5d9a85bcf72dadabb6262ecaeea3fc382ed529887d24db71f8582e9bf2e3914ce7d363",
    ),
    (
        34,
        "f231d1de5ab3384fa08347ba687d3838e502f8a423e0886978a465bca828483817799c0e8e3c0dd8dba39879fb8f6fc4",
    ),
    (
        35,
        "c87377ce11c2f2e85823e16f65546eba735611076e69f9c87487dc2d99a100ebb1c61406aa9365b8a7bb4bb9099f5950",
    ),
    (
        36,
        "544b20e728b062ed586ae5d0e83cc8fce9e89e5fc9b8904f6003152d05f908107d178d298121108e2fd367898ab94698",
    ),
    (
        37,
        "2cb533e5a87eebac0dce340d451270e1089148fce1cc2997824e62997c3b8bd587832c10283935ee9b056c7ee1240ad7",
    ),
    (
        38,
        "9ca05fd221f6efa0976586b8f0f19a4b86a0090750e5865fc3881bece88eef9b2bbffa57a12320fc2c4dc9c1e20d8161",
    ),
    (
        39,
        "b509f7a5f861b4c30f3817414b87c331fce574f637ff77df3ae3bec76d063d65303a866fd71650fcdf43db987a792855",
    ),
    (
        40,
        "fecb4edddaeac6ae9ffe74e71ba6d17c81480337a6908f4b28e5f125566fa48f242e947474fb0c2f8f06667dc445102b",
    ),
    (
        41,
        "8b0ba930edb9afc18a644248d24cea3281014034db8793cfc54e8b4176b4f5a5645fb96515e1735dc5c50e93b343a8e3",
    ),
    (
        42,
        "4e0ba92d532d47e62d25f373d0d132774edc32ac8fedcd8ef15a3ced6a99a3fc879dd580521d22b52bdc137fc05a701d",
    ),
    (
        43,
        "3622e28fd1e18cbd6579f6f5e8fa405038df0bab96c1e7ab2ad6e33796d42565ad20e99167ff1dccb33a528c7a76f9a8",
    ),
    (
        44,
        "1bf94ac5f84425b159903b1a3132a87d7a0c7a537ec67bda883be73346054b4a611eca9542b95103f20727907aab462e",
    ),
    (
        45,
        "84e340d5e4bce46f09d5cec690ab89874c6ad5796009c7f15c6cbbe8b8f17f5f2696f7052ee9a1fcc9a134c8010ed631",
    ),
    (
        46,
        "77f112826bfe33f2f255dfeeb42b961f3ac8efef4a153eedd71fa85ed59254dd8994e9b526bcf4b707fef145383fb6ab",
    ),
    (
        47,
        "4aea317c0cf3a3a3291821f9c94415304125f583aac590aea4d36a9dcd8983197223083e3ab9fb1aeff0c1201aa0c8ef",
    ),
    (
        48,
        "290cd348244f40bbfb134d852378f88eac57f3c7695012b39ce018ea477c1f509242c61b9776134f0d5334e9229871b2",
    ),
    (
        49,
        "7af9a873d1d3e5a9a5e5b66e8616a3d5b420d5d39433e040bbe94e8fb093f5a4fcdd115fa0aa46798ae646476fca19cf",
    ),
    (
        51,
        "0949bd64557d325001b966b9a3d02336bcfc0ed108f3b876532bd231d640788a3a014d78e2fe754ef971c868c6baeb21",
    ),
    (
        52,
        "ef904f333268ff67d102282948c16a3f2d8e582d9bb9cb508b86f32dc1039240c7f5fed462313ae0980cc39c41a309ba",
    ),
    (
        53,
        "d1c4ce1b06cc6031102944622a81dc03a32e0cd0637fe4104784beb5f42306c91c17563788acb11681fcf4a7a1001e7a",
    ),
    (
        54,
        "c943d0983cd6ec822a97c6460eb180669aa534e07bef8ad4c6daa46ab3040ef4f7e3af14081ffab6d8dca3071666c727",
    ),
    (
        55,
        "85c1eff93b58a5c889e5cdec32f9cdc1e8e0f19356f55e063f55a9a4144e1fb031e2ac21b8030afd64e604d8ba919bf5",
    ),
    (
        56,
        "38903df4663ed6dcd42c1c4a9ae7d2609b6780a40fc90385456dff6467357e3042d1024b8dbc6a950eedee07f9b356a6",
    ),
    (
        58,
        "5321d4b102bec093f8cf969867760b5145cdb70dfe8e420f917dcb0041273e1709b4318fac4953826ddbb492520f463e",
    ),
    (
        59,
        "727d776d888c144e05cb4d3eda38ae9d8590cc30667653cc3690ae4bc5676f0ec0601956c640388bb1f1952c35c6bddd",
    ),
    (
        60,
        "420b550f4648bfe94f4860401001903d4f23b5ed6c61fa3ff37dbaaf430b1b73b1097e67064acec6fa3562c65fcc25d6",
    ),
    (
        61,
        "6691d011593cbd72c92482e55a322c7ff4852891a9e808a063f96d7089f3f0d15c94a401b280ecbf571faa2fbc048205",
    ),
    (
        62,
        "6a5b1107e62835f3ab54587c676b5daa988613033986a5edcec23e01ce2eb65e2b264e854341927e1b78d96e1b56d666",
    ),
    (
        63,
        "cd640d884c914b9cc056ba8d0bcce1f941db30997fc063eb79223a089e5cdb9bcf2214679b7303b9ec80941281a0b4de",
    ),
    (
        64,
        "120bfa45ca64301341bc38a0f3675da19092da76a2586a66e3f902dded936b860f66f7a8992d3037d121c3718e8aa5a4",
    ),
    (
        65,
        "241f4faf76ef359ab32bd6310a84f5e02210b3811ee6f182b2a4c8c78d776590b70a31811bf80dfeec45ff1262f59755",
    ),
    (
        66,
        "aa4d4f54ea9fa0450c9cd39e464e16f3060b7ee90f106c533701a548b23085b29e4a01476e295cdb5beb68716b0e8544",
    ),
];

async fn fresh(dir: &tempfile::TempDir) -> (String, sqlx::SqlitePool) {
    let url = format!("sqlite://{}", dir.path().join("pulse.db").display());
    let db = pulse_server::db::connect(&url).await.unwrap();
    (url, db)
}

async fn recorded(db: &sqlx::SqlitePool) -> Vec<(i64, Vec<u8>)> {
    sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn checksums_match_what_sqlx_0_8_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (_, db) = fresh(&dir).await;
    let rows = recorded(&db).await;
    for (version, old) in RECORDED_BY_SQLX_0_8 {
        let (_, now) = rows
            .iter()
            .find(|(v, _)| v == version)
            .unwrap_or_else(|| panic!("migration {version} is no longer applied"));
        assert_eq!(
            &hex::encode(now),
            old,
            "migration {version}'s checksum changed"
        );
    }
}

/// The rule both versions follow (SHA-384 of the file's exact bytes), so migrations added after
/// the fixture are covered too.
#[tokio::test]
async fn checksums_are_sha384_of_the_files() {
    let dir = tempfile::tempdir().unwrap();
    let (_, db) = fresh(&dir).await;
    let migrations = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files = 0;
    for entry in std::fs::read_dir(&migrations).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some((version, _)) = name.split_once('_') else {
            continue;
        };
        let version: i64 = version.parse().unwrap();
        let sum = Sha384::digest(std::fs::read(&path).unwrap()).to_vec();
        let rows = recorded(&db).await;
        assert!(
            rows.contains(&(version, sum)),
            "{name}: recorded checksum isn't the file's SHA-384"
        );
        files += 1;
    }
    assert_eq!(files, recorded(&db).await.len());
}

/// A database whose table holds exactly the 0.8.6 rows migrates as a no-op, as production will.
#[tokio::test]
async fn a_database_migrated_by_sqlx_0_8_migrates_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let (url, db) = fresh(&dir).await;
    let latest = recorded(&db).await.last().unwrap().0;
    for (version, old) in RECORDED_BY_SQLX_0_8 {
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(hex::decode(old).unwrap())
            .bind(version)
            .execute(&db)
            .await
            .unwrap();
    }
    db.close().await;
    assert_eq!(pulse_server::db::migrate(&url).await.unwrap(), latest);
}
