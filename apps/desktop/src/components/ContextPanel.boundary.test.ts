import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import * as ts from "typescript";
import { describe, expect, it } from "vitest";

const INTERNAL_MODULES = [
  "contextAngleSteps",
  "contextAlignFold",
  "contextTechniques",
  "contextPaperDisplay",
] as const;

const PRODUCT_FILES = [
  "ContextPanel.tsx",
  ...INTERNAL_MODULES.map((name) => `${name}.tsx` as const),
] as const;

const EXPECTED_INTERNAL_EXPORTS = {
  "contextAngleSteps.tsx": [
    "FoldControls",
    "NumberInput",
    "RelaxationMessages",
    "StepContent",
  ],
  "contextAlignFold.tsx": [
    "AlignDraftContent",
    "AlignStartRow",
    "FoldDraftContent",
    "FoldThroughProposalContent",
  ],
  "contextTechniques.tsx": ["TechniqueDraftContent"],
  "contextPaperDisplay.tsx": [
    "CurveRow",
    "FoldAllPreviewContent",
    "LINE_TOOLS",
    "PullContent",
    "SelectionContent",
  ],
} as const;

/** C10〜C13着手直前の29関数。関数本体だけを比較し、export付与や移動先は許容する。 */
const BASELINE_FUNCTION_BODY_HASHES = {
  clampAngle: "90fff20898d664e5fec81e030ae071ed974e88cb111da6f66a247902da4821b8",
  completeNumber: "e3dcc8407c3570bc82a315c0754e5391607caa4132f97b9927239b969010a0fe",
  AngleNumberInput: "2b671482405d462ec528aacd2781f374bcc18bc5cd2f7fc87e46fe466a73ad2a",
  PinMark: "7c0ddab27cd341afaf38c9d12118aa886b994e0415aa43db1e760fe425ed9a25",
  HingeAngle: "70f4e2f6d7f48085afe3205b8277d2b80989fe0fe943d8b687917f08856eac17",
  HingeAngleGroup: "3ee75738c3fbae19bcc042b9d1140a1e7bc196ee7f22007894ce5780f3ea6b55",
  PoseRecordButton: "34af610326329f90b6ac091faf612012f3f41c095be2235ae6b9c62adcfd7385",
  FoldControls: "54c6bc3cc68098c26873e51fba36e4182b386bb82a8e5f81ff6f37d233cf19d9",
  NoteInput: "8a02a4acde38a272c4c131a33904cc0de48cd043b9b0539570473069fca11e5b",
  StepContent: "8d44729207be32e996f6fdd067e5305ee8853d0799bf399d89103973745ddf05",
  AlignStartRow: "6fc665317013b3edb22e9115a119ad5f8e5aa3938e75a1831ddf35f116345a47",
  AlignDraftContent: "4afcee3637082b2c9dc5bfaf3e535712c7de2291d2c0eb5e1c7f889c68b9f8ee",
  FoldDraftContent: "3816ccbcae0089ac9343bd46f6da5ad8cb9823838918e9c8b9f993f736d6bc5e",
  FoldThroughProposalContent:
    "f8a21e2d0822ab100102e9f048439bf5804a5bc14ebea34e4af7db63c6a913b6",
  NumberInput: "41a9968b9562363e6c00de91ab83be0fc6a376ed78b3882bee985fc14ff9b6d7",
  TwistPolygonRow: "a8629013d6354ac3efe70a64abee5d96e4241df557d722541a5044fd01742e1b",
  TechniqueLayerPicker:
    "9c0c69d9c31e790e57ea6dd03132558a4e370266a07ca936c8c65f9941e4ca0b",
  techniqueReferenceLabel:
    "e35f97cba90c380e5ac01c9d614243f10b4c71abd31c57b6db1930f962f23895",
  TechniqueReferenceRow:
    "4fe7f61e884a85fb525a249b58cdef36fed6171f8e626727a88458f42e309d0b",
  LayerMotionDraftContent:
    "937de3ebeee4345915d5ad1287a4e8b6eaa54cd7bf7cf704093c8e828ac33e4b",
  TechniqueDraftContent:
    "db1f8d94a1c8011f028a38ee47f8684e725206f78ca4fa244bc48b544fb703e1",
  NamedTechniqueDraftContent:
    "e3b69a3e71a88589b9e158287c4f17d5ba40dd1b20401cc1f30eca3033057a21",
  PullContent: "e5b0550ea8703fed3086708cb457dc76344e6a156a310e1105f0e1235efabd4a",
  PaperActionEntrances:
    "08b734b6e09a152d15e4b43c2724c718bb4725115d6233c455602499408e75f2",
  FoldAllPreviewContent:
    "e413e7685e0f12b4eb005c5f2afaaaacf36efd3dfd5c0f301d8440f8ac4d6df5",
  CurveRow: "c936e1fbd83c3c0bc8fa6266373e792cac8aff01e437c694523f35bef2d11650",
  SelectionContent:
    "9088782fd340d87b8ffbdb2d4671755e7265b141179612d16058a6dba4379d6a",
  RelaxationMessages:
    "8e36c4dd33169cc2588555b6ea1ccb5b2e09449b6f57b3e4e0e8d391780ea822",
  ContextPanel: "f5e59698bcc7b879e986616b74906adefde4c3313106183070f8acb7d305487e",
} as const;

/** useAppStoreへ渡していた102 selectorのSHA-256と重複数。 */
const BASELINE_SELECTOR_HASH_COUNTS: Record<string, number> = {
  "01af24c4de215dcbb4fb63cc177077dfe2a0fe49c2023546db332a69e00e5177": 1,
  "02dfc81884f4822848acfdb23251d125239b9da0e44d3931dce62841a5787efb": 3,
  "095d250c617e0a2350485d2ae052b1f68da837649f664592ccbb3d23560133aa": 2,
  "1761034848f4083c1104e05aa5e555fcb02cd9d54be9a2fc8cdb0618a7c9654b": 2,
  "2a6f29047de1156e91cb9d9c73924a6658f5821d79a0ba32f7c153fba63e5ddc": 1,
  "2c47a196e68bb1357ffeb60f11f130b0c7edd680647c3ea25b49df7c65748883": 1,
  "2e9ca86a9c1a53dc16f043cadcd8f85a3df8c1c752609675af260c44b88ddd44": 1,
  "2ee1e482f601451121edd59b984ec327f9ba808fc8dd0b35ffa8058345341956": 1,
  "3030641a9e477bef33068a97ce600e1d9f149456e9575f8585a64a3abe598e28": 1,
  "3078548d39dbbd4432d99a4c8da1c2d0ff17867fa20e046db84e28f97316315d": 1,
  "3747610042f851fe7d169b9f269835e563a988b4821a53be077e0de0db2e79df": 2,
  "375235ce43caaf66e801ae49ba750b0366a57ec74b7ca062f62d8f088aff9b3f": 1,
  "42f15cfb3f647b3f7248d710a7e30ac7c95c2c35b7958e3bdbdbb36e89ea590b": 1,
  "43ed30ba8224073f9b0df1ed4e3248afe9da1ae7a1684414f7805648499689f5": 1,
  "450237984ed615cfefd4f536b283afcf2a974203b158602b8b0781c3359e0465": 1,
  "47e584963591bc2e431ccacf566dd8979439a59600efc19f95678bbf5eb871f0": 1,
  "4ac32333324c8238087d23a3ce28677ab59199c14a06dcc0afcfc76264824dd1": 1,
  "4d071766fd6ad0461c5d91f4adead3802137eecd19122b9dd93cde6460f83048": 2,
  "4e3f546115401df5bc3e6bcca09dbc589b44c69aadc8eb944994cc2ec696d892": 1,
  "4ec687f393224a43d1492e4b2be8819bb94f92eb742fea33713ecabe4d1367ac": 2,
  "53f520900f6eb98e3568424b27b833c706ac92a6c80ae067ab238d64629cad41": 1,
  "57d264bdbf2a7de0a43a2cb5db5b3b4cbdabd72338addb0d95890018a8fcaf95": 1,
  "5a07aeffcf6d252ba37c9b8bbd511f88b200eefe1b83948bef2f9d45968888b7": 1,
  "5a5aceb9ecf9877cb321c9f3dc5bdf52e7234752c1945a0e2c77f49c198b12c6": 1,
  "5cbfd5af7ed765b9be9c0fafa210efda17619f2a44fcc58045d7613d4f573b93": 1,
  "5e263940995999f01e2308050f1f0c932a35511f72f304a2684214401b467969": 1,
  "611668b36f453ff4bf1d01118471e4f80b149dd6a7ce1ded4cb1f908f0895f26": 1,
  "61886be6eb8397e45baf934b695193117e91a6c0ce922a083ae8be4a8360bf27": 1,
  "650b53b9830040d3f6b78efd26f7fe5842129f6b8953034ee0f6fb4ccf807a83": 2,
  "66ca34f92f98887177095e0c06f4d1c82051e03c8b6717036dc1fd309eea9d6a": 1,
  "693302dcbf4d8c169805131c96ac365fae5beac5364306988b6bd703c8dbdb56": 3,
  "6b721084cac6dff609e7a5442d5741a65d201c1fd787a6cbf6a179a08f856fe3": 1,
  "6eb6df8eedbb6691fda2c4da417c313cb5f1292f0a2036845656f9080f554350": 1,
  "71c448e5874eccc2a61051e273888129dcd9d19a96617b2786992b23a560d826": 2,
  "755c92a83c23f54a5aee9575f1dd636d072b21197aa379f14106c0551d6151a5": 2,
  "783a75706036262288ab1e73728d9d73dafaf6c748b7a4751b669fc9aa5cf643": 1,
  "78de5ef6ee9d512d840e5c98058cc3fadf8949a0e930ec3f0be13db3cead7735": 2,
  "7f739b4c0e197d1e77523abd7ff02b48908e3074e9dee8d42e9e8f152ae568da": 1,
  "8030ff6950b36724b7e2cc51f4e43b7e98f140ad3fb4d58e5176462b8d42d0a6": 1,
  "811dc97c6d5c9b4bcef4d1856f9a3b0631ac7828829a60a7efa265903876a289": 1,
  "84a980e6255510ee400cddf2c889d6f6e9d451dded60b55878c29613dd41ee28": 1,
  "86f16dd3171074c3bca0c856a6e1321c036bed33b70d3791a3b1100400a62d51": 1,
  "879cd0f092d7e0e660faba9e96f806f938244d4c40cea09bfdce04b85da64272": 1,
  "9171f462a61ec29bbc29b76c39f6515acaa9247a1201be06e64fd367e77521d3": 2,
  "931675f5cce1ff088f57d0bc23c6acdeb7021e63a38b4c7e09bb19d9a4af1543": 1,
  "941eeea81be3164e0d7d2a30a834ccbaa276722aae5cb22308950fa2a6fab3e3": 1,
  "9942b771c15695e89cd95ce2efb81fe482cb643d098cbc30cff75eb5277f299d": 1,
  "99adbde326f8d14ef024f8a35c4ee0263f26886217d9182534aaf2b7c8981dfc": 1,
  "9bc60eea58b6674219f80f49bada5f7ef9841bef789e6e87d01957348b0f6dfa": 1,
  "9e4ad0fdf5da11e673346869acead82a7cf6bc7809a801fb79e381d190239ae2": 1,
  "a29b7a7106df3d5134f58c9031593260109e109f1aad911e725334ecf939806f": 1,
  "a383f02a36a8496b076f957c5c01aaf1a179b384265bc6808e5c428bf58341a3": 1,
  "ab0e99fadbec495a3dbaef6c73c7ec469705b940e5c41b337ed428f21d78c483": 1,
  "ab62ecf4d9ce5c992c894a561448e39c38a2e5d5b5afa96969affdcd9e991dc7": 1,
  "ad9fb32c5d1b269b1d10f0bfb099f06d550de40747ce447a871cc33b71fccc60": 1,
  "b0fcd4214db5d24a54cc7f3802547b1cd5992d13c9f7cb5a8d019616afc38c89": 1,
  "b19b8ae3244ba4fa1e61c35826f4ab606ad93a1af8eecfa1eb451d3ef7fa5ff0": 1,
  "bff4986d7b8ea816d12792782dbd6ba6249516612467344f78b6047ba986cbd0": 2,
  "c594753c9c67570763c6be9bc30a38b4840919c07abf60b8ea2fed57226138eb": 3,
  "c5f67eb880cdc73e92a2687bf08012bbfafa6df4cac20ad330dedd9684e1fa29": 1,
  "c6e5a3b0d12f555e494c4f9f58ac995d64a22a9c0ba967b3b157e229a927d050": 2,
  "c7062fa6330ee670135880a3f32314c834364cd11874099a687bae2038e3fa8a": 1,
  "d47bea89ea5d62c6e5cba254af0069f35f2c20c318bc34644b4234e940607495": 2,
  "db882a60c31c239f59132fd0a46d9b126f6d7ded9e8b836cf0629cf218fe684e": 1,
  "e1b2550ea851867d8d60e56bcfb9c0ec9400e1c9e2b08097d8d09639f0f95e96": 1,
  "e3ca4b32c1537ca7b97208c4ff8b6122c64fb4924e88b2ccb9902b6587213e31": 1,
  "eb403885c47466242546f9908b6f457672fc6407e333016504e044b04016f6ed": 3,
  "efae4b2224a39b7c9306d52c5dc510a949efc66ef24e027e738c8d4f7fe0c101": 3,
  "efd064ec03187bbe99c1c5c27369b1afa822948958f16b8f966621a7e7b8dcc7": 1,
  "f026e17194dd0ea34ecef6b73edbfa361eada466ccf46f520a93b1c592e1d139": 4,
  "f1f918168455b9b824cf79fe6f857f502186de0cf2e090ce0d66302e2682f1f3": 1,
  "f20bbba55faef8b5f057762d9e5e7c05e3218ffa501b18fb014678cf0e89d14b": 2,
  "f4731a53e40b1c603533b2996a08156048bb51ee47b2b5e83ee21b91867c6069": 1,
  "fb8b226c092b8e7381d8c66fadaf0a69c8f574a8764dd51805717b25d1a6bd5d": 1,
  "fc00d2b087bed676ca003f06e6630633dc953bb846346e7b0c169b3bf1781c09": 1,
};

const BASELINE_CONST_INITIALIZER_HASHES = {
  KIND_LABEL: "452a34d0923c84f311c9fca2ac9da79659c13b107fa15a020d1d753ab6e9be62",
  LINE_TOOLS: "8e9ea6924e33f7aa807a1ae2ee105e90a58a63cceda55b4ead8f225a9eeef157",
  ANGLE_MIN: "d20b21a66eaee78f943feec5c51bc3440b766854bd6411dcd627080fb6b2540f",
  ANGLE_MAX: "7b69759630f869f2723875f873935fed29d2d12b10ef763c1c33b8e0004cb405",
  ALIGN_MODES: "0cc4b86ad5bbbcd0d3d08e71d623cc37e8576786ea598e2e83e9e76ff3a577a3",
} as const;

type ProductName = (typeof PRODUCT_FILES)[number];
type ParsedProduct = { name: ProductName; source: string; file: ts.SourceFile };

function productPath(name: ProductName): string {
  return fileURLToPath(new URL(name, import.meta.url).href);
}

function productExists(name: ProductName): boolean {
  try {
    readFileSync(productPath(name), "utf8");
    return true;
  } catch {
    return false;
  }
}

function productsExist(): boolean {
  return PRODUCT_FILES.every(productExists);
}

function parsedProducts(): ParsedProduct[] {
  return PRODUCT_FILES.map((name) => {
    const source = readFileSync(productPath(name), "utf8");
    return {
      name,
      source,
      file: ts.createSourceFile(name, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX),
    };
  });
}

async function sha256(text: string): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return [...new Uint8Array(digest)]
    .map((value) => value.toString(16).padStart(2, "0"))
    .join("");
}

function visit(node: ts.Node, callback: (node: ts.Node) => void): void {
  callback(node);
  ts.forEachChild(node, (child) => visit(child, callback));
}

function topLevelFunctions(products: ParsedProduct[]): Map<string, ts.FunctionDeclaration> {
  const found = new Map<string, ts.FunctionDeclaration>();
  for (const { file } of products) {
    for (const statement of file.statements) {
      if (ts.isFunctionDeclaration(statement) && statement.name && statement.body) {
        expect(found.has(statement.name.text), `duplicate function: ${statement.name.text}`).toBe(false);
        found.set(statement.name.text, statement);
      }
    }
  }
  return found;
}

function sourceFileOf(node: ts.Node): ts.SourceFile {
  return node.getSourceFile();
}

function classNameOf(element: ts.JsxElement): string | null {
  const attribute = element.openingElement.attributes.properties.find(
    (item): item is ts.JsxAttribute =>
      ts.isJsxAttribute(item) && item.name.getText() === "className",
  );
  return attribute && attribute.initializer && ts.isStringLiteral(attribute.initializer)
    ? attribute.initializer.text
    : null;
}

function findJsxElementByClass(node: ts.Node, className: string): ts.JsxElement {
  let found: ts.JsxElement | null = null;
  visit(node, (child) => {
    if (found === null && ts.isJsxElement(child) && classNameOf(child) === className) {
      found = child;
    }
  });
  if (found === null) throw new Error(`missing JSX class: ${className}`);
  return found;
}

function unwrap(expression: ts.Expression): ts.Expression {
  let current = expression;
  while (ts.isParenthesizedExpression(current)) current = current.expression;
  return current;
}

function exportedNames(file: ts.SourceFile): string[] {
  const exported: string[] = [];
  for (const statement of file.statements) {
    const hasExport = ts.canHaveModifiers(statement)
      ? ts.getModifiers(statement)?.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword)
      : false;
    if (hasExport && "name" in statement) {
      const name = (statement as { name?: ts.Node }).name;
      if (name && ts.isIdentifier(name)) exported.push(name.text);
    }
    if (hasExport && ts.isVariableStatement(statement)) {
      for (const declaration of statement.declarationList.declarations) {
        if (ts.isIdentifier(declaration.name)) exported.push(declaration.name.text);
      }
    }
    if (ts.isExportDeclaration(statement)) {
      if (!statement.exportClause || !ts.isNamedExports(statement.exportClause)) {
        exported.push("*");
      } else {
        exported.push(...statement.exportClause.elements.map((element) => element.name.text));
      }
    }
  }
  return exported.sort();
}

describe("Context C10〜C13の分割境界", () => {
  it("C10〜C13を4つの実装moduleへ分け、facadeから組み立てる", () => {
    const missing = PRODUCT_FILES.filter((name) => !productExists(name));
    expect(missing).toEqual([]);

    const facade = readFileSync(productPath("ContextPanel.tsx"), "utf8");
    for (const moduleName of INTERNAL_MODULES) {
      expect(facade).toMatch(new RegExp(`from\\s+["']\\./${moduleName}["']`));
    }
  });

  it("5製品fileは各1,500行以下で、useStateを導入しない", () => {
    if (!productsExist()) return;
    const products = parsedProducts();
    expect(
      Object.fromEntries(
        products.map(({ name, source }) => [name, source.split(/\r?\n/).length]),
      ),
    ).toEqual(
      Object.fromEntries(PRODUCT_FILES.map((name) => [name, expect.any(Number)])),
    );
    for (const { name, source } of products) {
      expect(source.split(/\r?\n/).length, name).toBeLessThanOrEqual(1_500);
      expect(source.match(/\buseState\b/g) ?? [], name).toEqual([]);
    }
  });

  it("外から見えるContextPanel facadeの公開APIはContextPanelだけに保つ", () => {
    if (!productsExist()) return;
    const products = parsedProducts();
    expect(exportedNames(products[0].file)).toEqual(["ContextPanel"]);
    for (const [name, expected] of Object.entries(EXPECTED_INTERNAL_EXPORTS)) {
      const product = products.find((item) => item.name === name);
      if (!product) throw new Error(`missing product: ${name}`);
      expect(exportedNames(product.file), name).toEqual([...expected].sort());
    }
  });

  it("移したtop-level function 29/29の本体SHA-256が変わらない", async () => {
    if (!productsExist()) return;
    const products = parsedProducts();
    const functions = topLevelFunctions(products);
    expect([...functions.keys()].sort()).toEqual(
      Object.keys(BASELINE_FUNCTION_BODY_HASHES).sort(),
    );
    const actual = Object.fromEntries(
      await Promise.all(Object.keys(BASELINE_FUNCTION_BODY_HASHES).map(async (name) => {
        const declaration = functions.get(name);
        if (!declaration?.body) throw new Error(`missing function body: ${name}`);
        return [name, await sha256(declaration.body.getText(sourceFileOf(declaration)))] as const;
      })),
    );
    expect(actual).toEqual(BASELINE_FUNCTION_BODY_HASHES);
  });

  it("useAppStore selector callback 102/102のSHA-256が変わらない", async () => {
    if (!productsExist()) return;
    const counts: Record<string, number> = {};
    const selectorTexts: string[] = [];
    for (const { file } of parsedProducts()) {
      visit(file, (node) => {
        if (
          ts.isCallExpression(node) &&
          ts.isIdentifier(node.expression) &&
          node.expression.text === "useAppStore" &&
          node.arguments[0] &&
          (ts.isArrowFunction(node.arguments[0]) || ts.isFunctionExpression(node.arguments[0]))
        ) {
          selectorTexts.push(node.arguments[0].getText(file));
        }
      });
    }
    for (const text of selectorTexts) {
      const hash = await sha256(text);
      counts[hash] = (counts[hash] ?? 0) + 1;
    }
    expect(selectorTexts).toHaveLength(102);
    expect(counts).toEqual(BASELINE_SELECTOR_HASH_COUNTS);
  });

  it("表示判断に使う重要const initializerを変えない", async () => {
    if (!productsExist()) return;
    const actual: Record<string, string> = {};
    for (const { file } of parsedProducts()) {
      for (const statement of file.statements) {
        if (!ts.isVariableStatement(statement)) continue;
        for (const declaration of statement.declarationList.declarations) {
          if (
            ts.isIdentifier(declaration.name) &&
            declaration.initializer &&
            declaration.name.text in BASELINE_CONST_INITIALIZER_HASHES
          ) {
            actual[declaration.name.text] = await sha256(
              declaration.initializer.getText(file),
            );
          }
        }
      }
    }
    expect(actual).toEqual(BASELINE_CONST_INITIALIZER_HASHES);
  });

  it("internalからfacadeへの逆importと循環依存を0件に保つ", () => {
    if (!productsExist()) return;
    const moduleNames = PRODUCT_FILES.map((name) => name.replace(/\.tsx$/, ""));
    const graph = new Map(moduleNames.map((name) => [name, [] as string[]]));
    const reverseImports: string[] = [];
    for (const { name, file } of parsedProducts()) {
      const sourceName = name.replace(/\.tsx$/, "");
      for (const statement of file.statements) {
        if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) {
          continue;
        }
        const targetName = statement.moduleSpecifier.text.replace(/^\.\//, "");
        if (!graph.has(targetName)) continue;
        graph.get(sourceName)?.push(targetName);
        if (sourceName !== "ContextPanel" && targetName === "ContextPanel") {
          reverseImports.push(`${name} -> ${targetName}`);
        }
        if (
          sourceName !== "ContextPanel" &&
          targetName !== "ContextPanel" &&
          targetName !== "contextAngleSteps"
        ) {
          reverseImports.push(`${name} -> ${targetName}`);
        }
        if (
          targetName === "contextAngleSteps" &&
          sourceName !== "ContextPanel" &&
          sourceName !== "contextTechniques" &&
          sourceName !== "contextPaperDisplay"
        ) {
          reverseImports.push(`${name} -> ${targetName}`);
        }
      }
    }
    expect(reverseImports).toEqual([]);

    const visiting = new Set<string>();
    const visited = new Set<string>();
    const cycles: string[] = [];
    const walk = (name: string, path: string[]): void => {
      if (visiting.has(name)) {
        cycles.push([...path, name].join(" -> "));
        return;
      }
      if (visited.has(name)) return;
      visiting.add(name);
      for (const target of graph.get(name) ?? []) walk(target, [...path, name]);
      visiting.delete(name);
      visited.add(name);
    };
    for (const name of moduleNames) walk(name, []);
    expect(cycles).toEqual([]);
  });

  it("Contextの表示優先順と既存部品の再利用数を保つ", () => {
    if (!productsExist()) return;
    const products = parsedProducts();
    const functions = topLevelFunctions(products);
    const context = functions.get("ContextPanel");
    if (!context?.body) throw new Error("missing ContextPanel");
    const selection = findJsxElementByClass(context.body, "context-selection");
    const expression = selection.children.find(
      (child): child is ts.JsxExpression => ts.isJsxExpression(child) && child.expression !== undefined,
    )?.expression;
    if (!expression) throw new Error("missing context-selection branch");

    const conditions: string[] = [];
    let branch = unwrap(expression);
    while (ts.isConditionalExpression(branch)) {
      conditions.push(branch.condition.getText(branch.getSourceFile()));
      branch = unwrap(branch.whenFalse);
    }
    expect(conditions).toEqual([
      "foldAllPreview !== null",
      "pendingFoldThrough",
      'activeTool === "measure"',
      "selectedStep !== null",
      "techniqueDraft",
      "alignDraft",
      "foldDraft",
      "hasSelectedHinge",
    ]);

    const expectedReuse = {
      PaperAppearance: 1,
      MirrorAxisControls: 2,
      OperationSteps: 9,
      NumberStepper: 2,
      MeasureControls: 1,
    };
    const actualReuse = Object.fromEntries(
      Object.keys(expectedReuse).map((tag) => [tag, 0]),
    ) as Record<string, number>;
    for (const { file } of products) {
      visit(file, (node) => {
        if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
          const tag = node.tagName.getText(file);
          if (tag in actualReuse) actualReuse[tag] += 1;
        }
      });
    }
    expect(actualReuse).toEqual(expectedReuse);
  });

  it("FoldAllの4文、既存入口、一時差し替え、DOM順を保つ", () => {
    if (!productsExist()) return;
    const functions = topLevelFunctions(parsedProducts());
    const foldAll = functions.get("FoldAllPreviewContent");
    const entrances = functions.get("PaperActionEntrances");
    const context = functions.get("ContextPanel");
    if (!foldAll?.body || !entrances?.body || !context?.body) {
      throw new Error("missing FoldAll contract functions");
    }
    const foldAllBody = foldAll.body;

    const heading = findJsxElementByClass(foldAllBody, "fold-all-preview-heading");
    const headingText = heading.getText(heading.getSourceFile());
    for (const sentence of [
      "全部いっぺんに折ってみる",
      "これは仮の形です",
      "手順には記録されません。",
      "紙を順番に折った形ではないため、どの紙が上になるかは決まっていません。",
    ]) {
      expect(headingText).toContain(sentence);
    }

    const orderedSections = [
      "fold-all-preview-heading",
      "fold-all-preview-control",
      "button-row",
      "fold-all-preview-notices",
    ].map((className) => findJsxElementByClass(foldAllBody, className));
    expect(orderedSections.map((element) => element.getStart())).toEqual(
      orderedSections.map((element) => element.getStart()).sort((a, b) => a - b),
    );
    const returnRow = findJsxElementByClass(foldAllBody, "button-row");
    expect(returnRow.getText(returnRow.getSourceFile())).toContain("いつもの表示に戻る");

    const entranceText = entrances.body.getText(entrances.getSourceFile());
    expect(entranceText).toContain("enterFoldAllPreview");
    expect(entranceText).toContain("全部いっぺんに折ってみる");
    const contextSelection = findJsxElementByClass(context.body, "context-selection");
    const contextSelectionText = contextSelection.getText(context.getSourceFile());
    expect(contextSelectionText.indexOf("foldAllPreview !== null")).toBeLessThan(
      contextSelectionText.indexOf("pendingFoldThrough"),
    );
    expect(contextSelectionText).toContain("<FoldAllPreviewContent />");
  });
});
