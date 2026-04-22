📂 Project: ts-hwp-transpilerMaster Blueprint for Claude Code1. 기술 스택 (Tech Stack)Core Engine: Rust (based on rhwp)Target Runtime: 
WebAssembly (WASM) via wasm-packInterface/Glue: TypeScriptBuild Tool: Vite (for the Previewer)Parsing Strategy: AST-based Transpilation 
(not regex/text extraction)2. 아키텍처 파이프라인 (The Pipeline)Ingestion: HWP(Binary/OLE) 또는 HWPX(XML)를 rhwp 파서로 
로드.Normalization: 표(Table) 데이터를 $N \times M$ 가상 그리드 매트릭스로 정규화. (병합된 셀을 참조 인덱스로 채움)Semantic Analysis 
(핵심): 각 셀의 메타데이터(배경색, 폰트 가중치, 정렬, 테두리)를 기반으로 셀의 역할(Label vs Content) 부여.Transpilation: 분석된 
IR(Intermediate Representation)을 바탕으로 Markdown AST 생성.Output: 정밀하게 정제된 Markdown 텍스트 및 Web Preview용 렌더링 데이터 
출력.3. Claude Code를 위한 상세 지시서 (The Mission)Step 1: 환경 구축 및 의존성 분석rhwp 레포지토리를 서브모듈 또는 로컬 참조로 설정하고, 
Document, Section, Paragraph, ControlTable 구조체를 분석하라.wasm-bindgen을 통해 Rust의 AST 구조체를 TS 인터페이스로 손실 없이 넘기기 
위한 Bridge Layer를 설계하라.Step 2: 표 시맨틱 추출 알고리즘 (Semantic Table Decomposition)가장 중요한 단계다. 다음 알고리즘을 
src/semantics/table.rs에 구현하라:Grid Map 생성: row_span, col_span을 해석하여 물리적 좌표($x, y$)를 논리적 좌표($i, j$)로 매핑하는 
TableGrid 구조체를 만들어라.Heuristic Labeling: * Cell.property.background_color가 특정 임계값보다 어둡거나 노란색 계열이면 Label로 
분류.Cell.paragraph.property.alignment가 Center이면서 텍스트 길이가 짧으면 Header 후보로 분류.셀 내부 텍스트가 ■, ○, ▶ 등으로 
시작하거나 :을 포함하면 Key-Value Pair로 분리.Structure Rebuilding: * 복잡한 병합 표는 표준 MD 표 대신 Definition List나 Nested Bullet 
List로 변환하는 로직을 기본값으로 하라.표 안의 표(Nested Table)는 재귀적으로 처리하여 계층 구조를 유지하라.Step 3: Transpiler 구현 
(Writing Logic)단순한 String Concatenation이 아닌, Markdown AST를 먼저 생성하고 이를 최종적으로 문자열화하라.국가 계획서 특유의 '조잡한 
레이아웃 표'는 과감하게 리스트 형태로 Transpile하여 가독성을 확보하라.4. 제약 및 준수 사항 (Strict Rules)No <table> tags: HTML 태그는 
절대 사용하지 않는다. 오직 Pure Markdown 문법만 사용하되, 필요시 ASCII Grid 형태를 옵션으로 제공한다.Memory Efficiency: WASM 환경에서 
실행되므로 대용량 파일 처리 시 Clone을 최소화하고 Reference를 활용하라.Edge Case: 0x0 크기의 셀, 깨진 OLE 스트림, 암호화된 배포용 문서에 
대한 예외 처리를 반드시 포함하라.
📂 Project: ts-hwp-transpiler (Ultra-Fidelity Version)Master Engineering Specification for Claude Code1. Mission ObjectiveHWP/HWPX 문서를 Semantic Markdown으로 트랜스파일하되, 원본의 **시각적 정밀도(그리기)**와 **논리적 구조(읽기)**를 1:1에 가깝게 보존한다. 특히 마크다운 출력물에는 폰트, 크기, 정렬 정보를 포함한 **'Hinting Layer'**를 삽입하여 차세대 문서 도구에서 재현 가능하도록 설계한다.2. 핵심 기술 스택 및 요구사항Engine: rhwp (Rust) → WebAssembly (WASM)Rendering High-Fidelity:Images: 바이너리 데이터 추출 후 DataURI 또는 Blob URL로 변환 매핑.Formulas: HWP 고유 수식 스크립트를 파싱하여 LaTeX로 변환 (MathJax/KaTeX 호환).Fonts: 원본 폰트명, 크기(pt), 자간, 장평 정보를 AST에 포함.Semantic Transpilation with Hinting:Markdown Hinting: 표준 마크다운 문법을 해치지 않도록 각 요소 상단에 `` 형태의 JSON 힌트 삽입.Table Precision: $N \times M$ 정규화 그리드를 기반으로 시맨틱 라벨링 수행.3. Claude Code 상세 태스크 (Step-by-Step)Step 1: AST 확장 및 메타데이터 캡처rhwp의 Paragraph 및 CharShape 구조체를 분석하여 아래 정보를 추출하는 MetadataExtractor를 구현하라.Font: 폰트 패밀리(한글/영문 분리), 글자 크기(milli-pt), 글자 색상.Paragraph: 정렬(Left/Center/Right/Justify), 줄간격, 내어쓰기/들여쓰기 값.HWP 바이너리 내 BinData 섹션에서 이미지를 추출하여 Uint8Array로 WASM 메모리에 올리는 로직을 확보하라.Step 2: 수식(Formula) 트랜스파일러 엔진HWP 수식 제어 문자(Equation)를 스캔하여 LaTeX 문법으로 변환하는 파서를 작성하라. (예: SUM -> \sum, ROOT -> \sqrt).변환된 LaTeX는 마크다운 내에서 $$...$$ 블록으로 감싸 출력한다.Step 3: 정밀 표 분석 및 시맨틱 맵핑 (The Brain)Grid Normalization: 모든 셀의 좌표를 계산하여 가상 매트릭스를 생성하고, 병합된 셀(span)을 원본 셀의 Pointer로 채워라.Semantic Labeling: * 배경색이 있는 셀, 굵은 글씨, 중앙 정렬된 첫 행/열을 **'Semantic Header'**로 정의.복잡한 표 구조를 분석하여 **'Key-Value 리스트'**로 변환할지 **'정규화된 MD 표'**로 출력할지 결정하는 임계값(Threshold) 로직을 구현하라.Step 4: Hinting 시스템이 포함된 MD Writer출력되는 각 문단과 표 앞에 시각적 힌트를 삽입하라.예시 출력:Markdown# 제1장 사업 개요

| 항목 | 내용 |
| :--- | :--- |
...
4. 개발 가이드라인 (Strict Constraints)WASM Layer: Rust에서 추출한 복잡한 객체를 TS로 넘길 때 데이터 직렬화 오버헤드를 줄이기 위해 serde-wasm-bindgen을 최적화하여 사용하라.Font Rendering: 웹 브라우저 프리뷰 시 HWP 시스템 폰트가 없을 경우를 대비해 Font-Fallback Map(함초롬바탕 -> NanumMyeongjo 등)을 정의하라.Table Fidelity: 표 내부의 리스트, 수식, 이미지가 포함된 경우에도 계층 구조를 깨뜨리지 말고 재귀적으로 트랜스파일하라.
